//! Target registration, server-side destination validation, and time-bounded
//! authorization records.

use axum::{
    Extension, Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use probeforge_contracts::{
    AuthorizationMethod, Environment, TargetLimits, TargetScope, validate_target_scope,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use probe_forge_target_policy::{
    ResolvedDestination, SystemResolver, TargetGuard, TargetPolicy, TargetValidationError,
    authorization_expiry, authorization_is_active, normalize_target_scope, resolve_destination,
    validate_authorization_limits,
};

use crate::{
    AppState, audit,
    auth::AuthContext,
    error::{ApiError, ApiJson, ApiPath},
    projects::{ProjectRole, load_project_role},
};

const MAX_LISTED_TARGETS: i64 = 100;
const MAX_LISTED_AUTHORIZATIONS: i64 = 20;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/projects/{project_id}/targets",
            get(list_targets).post(create_target),
        )
        .route("/targets/{target_id}", get(get_target))
        .route("/targets/{target_id}/validate", post(validate_target))
        .route(
            "/targets/{target_id}/authorizations",
            get(list_authorizations).post(create_authorization),
        )
        .route(
            "/target-authorizations/{authorization_id}/revoke",
            post(revoke_authorization),
        )
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TargetDto {
    id: Uuid,
    project_id: Uuid,
    normalized_origin: String,
    environment: Environment,
    allowed_hosts: Vec<String>,
    allowed_paths: Vec<String>,
    allowed_ports: Vec<u16>,
    verification_status: String,
    verification_checked_at: Option<DateTime<Utc>>,
    verified_addresses: Vec<String>,
    created_by: Uuid,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TargetDetailDto {
    target: TargetDto,
    authorizations: Vec<AuthorizationDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthorizationDto {
    id: Uuid,
    project_id: Uuid,
    target_id: Uuid,
    authorization_method: AuthorizationMethod,
    scope: TargetScope,
    limits: TargetLimits,
    confirmed_at: DateTime<Utc>,
    valid_from: DateTime<Utc>,
    expires_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
    active: bool,
}

#[derive(Debug, FromRow)]
struct TargetRow {
    id: Uuid,
    project_id: Uuid,
    normalized_origin: String,
    environment: String,
    allowed_hosts: Value,
    allowed_paths: Value,
    allowed_ports: Value,
    verification_status: String,
    verification_checked_at: Option<DateTime<Utc>>,
    verified_addresses: Value,
    created_by: Uuid,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct AuthorizationRow {
    id: Uuid,
    project_id: Uuid,
    target_id: Uuid,
    authorization_method: String,
    scope: Value,
    limits: Value,
    confirmed_at: DateTime<Utc>,
    valid_from: DateTime<Utc>,
    expires_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, FromRow)]
struct AuthorizationReference {
    project_id: Uuid,
    target_id: Uuid,
    revoked_at: Option<DateTime<Utc>>,
    expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateTargetRequest {
    origin: String,
    environment: Environment,
    #[serde(default)]
    allowed_hosts: Vec<String>,
    #[serde(default)]
    allowed_paths: Vec<String>,
    #[serde(default)]
    allowed_ports: Vec<u16>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateAuthorizationRequest {
    authorization_method: AuthorizationMethod,
    confirmed: bool,
    limits: TargetLimits,
    #[serde(default)]
    expires_at: Option<DateTime<Utc>>,
}

const TARGET_COLUMNS: &str = r#"
    id, project_id, normalized_origin, environment, allowed_hosts, allowed_paths,
    allowed_ports, verification_status, verification_checked_at, verified_addresses,
    created_by, created_at, updated_at
"#;

const AUTHORIZATION_COLUMNS: &str = r#"
    id, project_id, target_id, authorization_method, scope, limits, confirmed_at,
    valid_from, expires_at, revoked_at
"#;

async fn list_targets(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(project_id): ApiPath<Uuid>,
) -> Result<Json<Vec<TargetDto>>, ApiError> {
    load_project_role(&state.pool, context.user.id, project_id).await?;
    let query = format!(
        "SELECT {TARGET_COLUMNS} FROM targets WHERE project_id = $1 \
         ORDER BY created_at DESC LIMIT $2"
    );
    let rows: Vec<TargetRow> = sqlx::query_as(sqlx::AssertSqlSafe(query))
        .bind(project_id)
        .bind(MAX_LISTED_TARGETS)
        .fetch_all(&state.pool)
        .await
        .map_err(ApiError::database)?;
    Ok(Json(
        rows.iter().map(target_dto).collect::<Result<Vec<_>, _>>()?,
    ))
}

async fn create_target(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(project_id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<CreateTargetRequest>,
) -> Result<(StatusCode, Json<TargetDto>), ApiError> {
    let role = load_project_role(&state.pool, context.user.id, project_id).await?;
    require_target_writer(role)?;

    let scope = normalize_target_scope(
        &payload.origin,
        payload.environment,
        &payload.allowed_hosts,
        &payload.allowed_paths,
        &payload.allowed_ports,
        &state.target_policy,
    )
    .map_err(target_validation_error)?;
    validate_target_scope(&scope)
        .map_err(|_| target_validation_error(TargetValidationError::InvalidScope))?;

    let mut transaction = state.pool.begin().await.map_err(ApiError::database)?;
    let row: Option<TargetRow> = sqlx::query_as(
        r#"
        INSERT INTO targets (
            project_id, normalized_origin, environment, allowed_hosts,
            allowed_paths, allowed_ports, verification_status, created_by
        ) VALUES ($1, $2, $3, $4, $5, $6, 'pending', $7)
        RETURNING id, project_id, normalized_origin, environment, allowed_hosts,
                  allowed_paths, allowed_ports, verification_status,
                  verification_checked_at, verified_addresses, created_by,
                  created_at, updated_at
        "#,
    )
    .bind(project_id)
    .bind(&scope.origin)
    .bind(environment_to_db(payload.environment))
    .bind(json!(scope.allowed_hosts.clone()))
    .bind(json!(scope.allowed_paths.clone()))
    .bind(json!(scope.allowed_ports.clone()))
    .bind(context.user.id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(map_target_write_error)?;
    let Some(row) = row else {
        return Err(ApiError::database(sqlx::Error::RowNotFound));
    };

    audit::record_in_tx(
        &mut transaction,
        Some(project_id),
        Some(context.user.id),
        "target.created",
        "target",
        Some(row.id),
        json!({
            "origin": row.normalized_origin.clone(),
            "environment": row.environment.clone(),
            "allowedHostCount": scope.allowed_hosts.len(),
            "allowedPathCount": scope.allowed_paths.len(),
            "allowedPortCount": scope.allowed_ports.len(),
        }),
    )
    .await
    .map_err(ApiError::database)?;
    transaction.commit().await.map_err(ApiError::database)?;

    tracing::info!(
        event = "target.created",
        target_id = %row.id,
        project_id = %row.project_id,
        user_id = %context.user.id,
        environment = %row.environment,
        "target registered"
    );
    Ok((StatusCode::CREATED, Json(target_dto(&row)?)))
}

async fn get_target(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(target_id): ApiPath<Uuid>,
) -> Result<Json<TargetDetailDto>, ApiError> {
    let row = load_target_for_user(&state.pool, context.user.id, target_id).await?;
    let authorizations = load_authorizations_for_target(&state.pool, target_id).await?;
    Ok(Json(TargetDetailDto {
        target: target_dto(&row)?,
        authorizations,
    }))
}

async fn validate_target(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(target_id): ApiPath<Uuid>,
) -> Result<Json<TargetDto>, ApiError> {
    let role = load_target_role(&state.pool, context.user.id, target_id).await?;
    require_target_writer(role)?;
    let row = load_target(&state.pool, target_id).await?;

    let destination = match revalidate_target(&row, &state.target_policy).await {
        Ok(destination) => destination,
        Err(error) => {
            persist_failed_validation(&state, &row, context.user.id, &error).await?;
            return Err(target_validation_error(error));
        }
    };
    persist_verified_validation(
        &state,
        &row,
        context.user.id,
        &destination,
        "explicit_validation",
    )
    .await?;
    let updated = load_target(&state.pool, target_id).await?;
    Ok(Json(target_dto(&updated)?))
}

async fn list_authorizations(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(target_id): ApiPath<Uuid>,
) -> Result<Json<Vec<AuthorizationDto>>, ApiError> {
    load_target_for_user(&state.pool, context.user.id, target_id).await?;
    Ok(Json(
        load_authorizations_for_target(&state.pool, target_id).await?,
    ))
}

async fn create_authorization(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(target_id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<CreateAuthorizationRequest>,
) -> Result<(StatusCode, Json<AuthorizationDto>), ApiError> {
    let role = load_target_role(&state.pool, context.user.id, target_id).await?;
    require_target_writer(role)?;
    if payload.authorization_method != AuthorizationMethod::ExplicitConfirmation {
        return Err(ApiError::invalid(
            "unsupported_authorization_method",
            "Only explicit confirmation is supported in this phase.",
        ));
    }
    if !payload.confirmed {
        return Err(ApiError::invalid(
            "confirmation_required",
            "Explicit target authorization confirmation is required.",
        ));
    }
    validate_authorization_limits(&payload.limits).map_err(target_validation_error)?;

    let row = load_target(&state.pool, target_id).await?;
    if row.verification_status != "verified" {
        return Err(ApiError::conflict(
            "target_not_verified",
            "Validate the target before creating an authorization.",
        ));
    }
    let destination = match revalidate_target(&row, &state.target_policy).await {
        Ok(destination) => destination,
        Err(error) => {
            persist_failed_validation(&state, &row, context.user.id, &error).await?;
            return Err(target_validation_error(error));
        }
    };
    persist_verified_validation(
        &state,
        &row,
        context.user.id,
        &destination,
        "authorization_revalidation",
    )
    .await?;
    let scope = target_scope_from_row(&row).map_err(target_validation_error)?;

    let now = Utc::now();
    let expires_at =
        authorization_expiry(payload.expires_at, now).map_err(target_validation_error)?;
    let limits_json = serde_json::to_value(&payload.limits)
        .map_err(|_| ApiError::internal("authorization limits could not be serialized"))?;
    let scope_json = serde_json::to_value(&scope)
        .map_err(|_| ApiError::internal("authorization scope could not be serialized"))?;

    let mut transaction = state.pool.begin().await.map_err(ApiError::database)?;
    let authorization: AuthorizationRow = sqlx::query_as(
        r#"
        INSERT INTO target_authorizations (
            project_id, target_id, authorized_by, authorization_method, scope, limits,
            confirmed_at, valid_from, expires_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $7, $8)
        RETURNING id, project_id, target_id, authorization_method, scope, limits,
                  confirmed_at, valid_from, expires_at, revoked_at
        "#,
    )
    .bind(row.project_id)
    .bind(row.id)
    .bind(context.user.id)
    .bind(authorization_method_to_db(payload.authorization_method))
    .bind(&scope_json)
    .bind(&limits_json)
    .bind(now)
    .bind(expires_at)
    .fetch_one(&mut *transaction)
    .await
    .map_err(ApiError::database)?;

    audit::record_in_tx(
        &mut transaction,
        Some(row.project_id),
        Some(context.user.id),
        "target.authorization_created",
        "target_authorization",
        Some(authorization.id),
        json!({
            "targetId": row.id,
            "origin": row.normalized_origin.clone(),
            "expiresAt": expires_at,
            "maxRequests": payload.limits.max_requests,
            "maxDurationSeconds": payload.limits.max_duration_seconds,
        }),
    )
    .await
    .map_err(ApiError::database)?;
    transaction.commit().await.map_err(ApiError::database)?;

    tracing::info!(
        event = "target.authorization_created",
        authorization_id = %authorization.id,
        target_id = %row.id,
        project_id = %row.project_id,
        user_id = %context.user.id,
        expires_at = %expires_at,
        "target authorization created"
    );
    Ok((
        StatusCode::CREATED,
        Json(authorization_dto(authorization, now)?),
    ))
}

async fn revoke_authorization(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(authorization_id): ApiPath<Uuid>,
) -> Result<Json<AuthorizationDto>, ApiError> {
    let reference: Option<AuthorizationReference> = sqlx::query_as(
        "SELECT project_id, target_id, revoked_at, expires_at
         FROM target_authorizations WHERE id = $1",
    )
    .bind(authorization_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(ApiError::database)?;
    let Some(AuthorizationReference {
        project_id,
        target_id,
        revoked_at,
        expires_at,
    }) = reference
    else {
        return Err(ApiError::not_found(
            "authorization_not_found",
            "Target authorization not found.",
        ));
    };
    let role = load_project_role(&state.pool, context.user.id, project_id).await?;
    require_target_writer(role)?;
    if revoked_at.is_some() {
        return Err(ApiError::conflict(
            "authorization_already_revoked",
            "The target authorization is already revoked.",
        ));
    }

    let now = Utc::now();
    let mut transaction = state.pool.begin().await.map_err(ApiError::database)?;
    let authorization: Option<AuthorizationRow> = sqlx::query_as(
        r#"
        UPDATE target_authorizations
        SET revoked_at = $2
        WHERE id = $1 AND revoked_at IS NULL
        RETURNING id, project_id, target_id, authorization_method, scope, limits,
                  confirmed_at, valid_from, expires_at, revoked_at
        "#,
    )
    .bind(authorization_id)
    .bind(now)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(ApiError::database)?;
    let Some(authorization) = authorization else {
        return Err(ApiError::conflict(
            "authorization_already_revoked",
            "The target authorization is already revoked.",
        ));
    };
    audit::record_in_tx(
        &mut transaction,
        Some(project_id),
        Some(context.user.id),
        "target.authorization_revoked",
        "target_authorization",
        Some(authorization_id),
        json!({
            "targetId": target_id,
            "expiresAt": expires_at,
        }),
    )
    .await
    .map_err(ApiError::database)?;
    transaction.commit().await.map_err(ApiError::database)?;

    tracing::info!(
        event = "target.authorization_revoked",
        authorization_id = %authorization_id,
        target_id = %target_id,
        project_id = %project_id,
        user_id = %context.user.id,
        "target authorization revoked"
    );
    Ok(Json(authorization_dto(authorization, now)?))
}

async fn load_target_role(
    pool: &PgPool,
    user_id: Uuid,
    target_id: Uuid,
) -> Result<ProjectRole, ApiError> {
    let project_id: Option<Uuid> =
        sqlx::query_scalar("SELECT project_id FROM targets WHERE id = $1")
            .bind(target_id)
            .fetch_optional(pool)
            .await
            .map_err(ApiError::database)?;
    let Some(project_id) = project_id else {
        return Err(ApiError::not_found("target_not_found", "Target not found."));
    };
    load_project_role(pool, user_id, project_id).await
}

async fn load_target_for_user(
    pool: &PgPool,
    user_id: Uuid,
    target_id: Uuid,
) -> Result<TargetRow, ApiError> {
    load_target_role(pool, user_id, target_id).await?;
    load_target(pool, target_id).await
}

async fn load_target(pool: &PgPool, target_id: Uuid) -> Result<TargetRow, ApiError> {
    let query = format!("SELECT {TARGET_COLUMNS} FROM targets WHERE id = $1");
    let row: Option<TargetRow> = sqlx::query_as(sqlx::AssertSqlSafe(query))
        .bind(target_id)
        .fetch_optional(pool)
        .await
        .map_err(ApiError::database)?;
    row.ok_or_else(|| ApiError::not_found("target_not_found", "Target not found."))
}

async fn load_authorizations_for_target(
    pool: &PgPool,
    target_id: Uuid,
) -> Result<Vec<AuthorizationDto>, ApiError> {
    let query = format!(
        "SELECT {AUTHORIZATION_COLUMNS} FROM target_authorizations \
         WHERE target_id = $1 ORDER BY confirmed_at DESC LIMIT $2"
    );
    let rows: Vec<AuthorizationRow> = sqlx::query_as(sqlx::AssertSqlSafe(query))
        .bind(target_id)
        .bind(MAX_LISTED_AUTHORIZATIONS)
        .fetch_all(pool)
        .await
        .map_err(ApiError::database)?;
    let now = Utc::now();
    rows.into_iter()
        .map(|row| authorization_dto(row, now))
        .collect()
}

async fn revalidate_target(
    row: &TargetRow,
    policy: &TargetPolicy,
) -> Result<ResolvedDestination, TargetValidationError> {
    let scope = target_scope_from_row(row)?;
    let environment = match row.environment.as_str() {
        "local" => Environment::Local,
        "staging" => Environment::Staging,
        "production" => Environment::Production,
        _ => return Err(TargetValidationError::InvalidScope),
    };
    let guard = TargetGuard { environment, scope };
    let resolver = SystemResolver;
    match tokio::time::timeout(
        policy.request_timeout,
        resolve_destination(&guard, &resolver, policy),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(TargetValidationError::DnsFailure),
    }
}

async fn persist_verified_validation(
    state: &AppState,
    row: &TargetRow,
    actor_id: Uuid,
    destination: &ResolvedDestination,
    source: &'static str,
) -> Result<(), ApiError> {
    let addresses: Vec<String> = destination
        .addresses
        .iter()
        .map(ToString::to_string)
        .collect();
    let previous = string_vec(row.verified_addresses.clone(), "verifiedAddresses")?;
    let changed = previous != addresses;
    let addresses_json = json!(addresses);

    let mut transaction = state.pool.begin().await.map_err(ApiError::database)?;
    sqlx::query(
        "UPDATE targets
         SET verification_status = 'verified',
             verification_checked_at = now(),
             verified_addresses = $2
         WHERE id = $1",
    )
    .bind(row.id)
    .bind(&addresses_json)
    .execute(&mut *transaction)
    .await
    .map_err(ApiError::database)?;
    audit::record_in_tx(
        &mut transaction,
        Some(row.project_id),
        Some(actor_id),
        "target.validated",
        "target",
        Some(row.id),
        json!({
            "origin": row.normalized_origin,
            "source": source,
            "addressCount": destination.addresses.len(),
            "addressesChanged": changed,
        }),
    )
    .await
    .map_err(ApiError::database)?;
    tracing::info!(
        event = "target.validated",
        target_id = %row.id,
        project_id = %row.project_id,
        source,
        address_count = destination.addresses.len(),
        addresses_changed = changed,
        "target destination validated"
    );
    transaction.commit().await.map_err(ApiError::database)
}

async fn persist_failed_validation(
    state: &AppState,
    row: &TargetRow,
    actor_id: Uuid,
    error: &TargetValidationError,
) -> Result<(), ApiError> {
    let mut transaction = state.pool.begin().await.map_err(ApiError::database)?;
    sqlx::query(
        "UPDATE targets
         SET verification_status = 'failed',
             verification_checked_at = now(),
             verified_addresses = '[]'::jsonb
         WHERE id = $1",
    )
    .bind(row.id)
    .execute(&mut *transaction)
    .await
    .map_err(ApiError::database)?;
    audit::record_in_tx(
        &mut transaction,
        Some(row.project_id),
        Some(actor_id),
        "target.validation_failed",
        "target",
        Some(row.id),
        json!({
            "origin": row.normalized_origin,
            "errorCode": error.code(),
        }),
    )
    .await
    .map_err(ApiError::database)?;
    tracing::warn!(
        event = "target.validation_failed",
        target_id = %row.id,
        project_id = %row.project_id,
        error_code = error.code(),
        "target destination validation failed"
    );
    transaction.commit().await.map_err(ApiError::database)
}

fn target_dto(row: &TargetRow) -> Result<TargetDto, ApiError> {
    Ok(TargetDto {
        id: row.id,
        project_id: row.project_id,
        normalized_origin: row.normalized_origin.clone(),
        environment: environment_from_db(&row.environment)?,
        allowed_hosts: string_vec(row.allowed_hosts.clone(), "allowedHosts")?,
        allowed_paths: string_vec(row.allowed_paths.clone(), "allowedPaths")?,
        allowed_ports: ports_vec(row.allowed_ports.clone())?,
        verification_status: row.verification_status.clone(),
        verification_checked_at: row.verification_checked_at,
        verified_addresses: string_vec(row.verified_addresses.clone(), "verifiedAddresses")?,
        created_by: row.created_by,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

fn target_scope_from_row(row: &TargetRow) -> Result<TargetScope, TargetValidationError> {
    let scope = TargetScope {
        origin: row.normalized_origin.clone(),
        allowed_hosts: serde_json::from_value(row.allowed_hosts.clone())
            .map_err(|_| TargetValidationError::InvalidScope)?,
        allowed_paths: serde_json::from_value(row.allowed_paths.clone())
            .map_err(|_| TargetValidationError::InvalidScope)?,
        allowed_ports: serde_json::from_value(row.allowed_ports.clone())
            .map_err(|_| TargetValidationError::InvalidScope)?,
    };
    validate_target_scope(&scope).map_err(|_| TargetValidationError::InvalidScope)?;
    Ok(scope)
}

fn authorization_dto(
    row: AuthorizationRow,
    now: DateTime<Utc>,
) -> Result<AuthorizationDto, ApiError> {
    let scope: TargetScope = serde_json::from_value(row.scope)
        .map_err(|_| ApiError::internal("stored authorization scope is invalid"))?;
    validate_target_scope(&scope)
        .map_err(|_| ApiError::internal("stored authorization scope is invalid"))?;
    let limits: TargetLimits = serde_json::from_value(row.limits)
        .map_err(|_| ApiError::internal("stored authorization limits are invalid"))?;
    validate_authorization_limits(&limits)
        .map_err(|_| ApiError::internal("stored authorization limits are invalid"))?;
    let active = authorization_is_active(row.valid_from, row.expires_at, row.revoked_at, now);
    Ok(AuthorizationDto {
        id: row.id,
        project_id: row.project_id,
        target_id: row.target_id,
        authorization_method: authorization_method_from_db(&row.authorization_method)?,
        scope,
        limits,
        confirmed_at: row.confirmed_at,
        valid_from: row.valid_from,
        expires_at: row.expires_at,
        revoked_at: row.revoked_at,
        active,
    })
}

fn string_vec(value: Value, field: &str) -> Result<Vec<String>, ApiError> {
    serde_json::from_value(value)
        .map_err(|_| ApiError::internal(format!("stored {field} is invalid")))
}

fn ports_vec(value: Value) -> Result<Vec<u16>, ApiError> {
    serde_json::from_value(value).map_err(|_| ApiError::internal("stored allowedPorts is invalid"))
}

fn environment_from_db(value: &str) -> Result<Environment, ApiError> {
    match value {
        "local" => Ok(Environment::Local),
        "staging" => Ok(Environment::Staging),
        "production" => Ok(Environment::Production),
        _ => Err(ApiError::internal("stored target environment is invalid")),
    }
}

fn environment_to_db(value: Environment) -> &'static str {
    match value {
        Environment::Local => "local",
        Environment::Staging => "staging",
        Environment::Production => "production",
    }
}

fn authorization_method_from_db(value: &str) -> Result<AuthorizationMethod, ApiError> {
    match value {
        "explicit_confirmation" => Ok(AuthorizationMethod::ExplicitConfirmation),
        "ownership_verification" => Ok(AuthorizationMethod::OwnershipVerification),
        _ => Err(ApiError::internal("stored authorization method is invalid")),
    }
}

fn authorization_method_to_db(value: AuthorizationMethod) -> &'static str {
    match value {
        AuthorizationMethod::ExplicitConfirmation => "explicit_confirmation",
        AuthorizationMethod::OwnershipVerification => "ownership_verification",
    }
}

fn require_target_writer(role: ProjectRole) -> Result<(), ApiError> {
    if role.can_manage_targets() {
        Ok(())
    } else {
        tracing::warn!(
            event = "target.write_forbidden",
            role = ?role,
            "target management denied: insufficient project role"
        );
        Err(ApiError::forbidden(
            "forbidden",
            "You do not have permission to manage targets in this project.",
        ))
    }
}

fn target_validation_error(error: TargetValidationError) -> ApiError {
    ApiError::invalid(error.code(), error.safe_message())
}

fn map_target_write_error(error: sqlx::Error) -> ApiError {
    if matches!(
        &error,
        sqlx::Error::Database(database_error)
            if database_error.code().as_deref() == Some("23505")
    ) {
        return ApiError::conflict(
            "target_exists",
            "A target with this normalized origin already exists in the project.",
        );
    }
    ApiError::database(error)
}

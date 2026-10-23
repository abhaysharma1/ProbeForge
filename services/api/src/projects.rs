use axum::{
    Extension, Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::{
    AppState,
    auth::{AuthContext, password::normalize_email},
    error::{ApiError, ApiJson, ApiPath, ApiQuery, AuditQuery},
};

const MAX_LISTED_PROJECTS: i64 = 100;
const MAX_LISTED_MEMBERS: i64 = 200;
const MAX_AUDIT_LIMIT: i64 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectRole {
    Owner,
    Admin,
    Collaborator,
    Viewer,
}

impl ProjectRole {
    pub(crate) fn from_db(value: &str) -> Option<Self> {
        match value {
            "owner" => Some(Self::Owner),
            "admin" => Some(Self::Admin),
            "collaborator" => Some(Self::Collaborator),
            "viewer" => Some(Self::Viewer),
            _ => None,
        }
    }

    fn can_manage_project(self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }

    fn can_manage_members(self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }

    fn can_view_audit(self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }

    /// Viewers are read-only. Every other project role may create and cancel
    /// analysis jobs.
    pub(crate) fn can_create_jobs(self) -> bool {
        !matches!(self, Self::Viewer)
    }

    /// Plan selection and approval are write operations. Viewers may review a
    /// plan but cannot change or approve it.
    pub(crate) fn can_review_plans(self) -> bool {
        !matches!(self, Self::Viewer)
    }

    /// Target registration, validation, authorization, and revocation are
    /// write operations. Viewers may read target state only.
    pub(crate) fn can_manage_targets(self) -> bool {
        !matches!(self, Self::Viewer)
    }

    /// Owner may manage admins, collaborators, and viewers. Admin may manage
    /// only collaborators and viewers. The owner role changes only through the
    /// dedicated ownership-transfer operation.
    fn can_manage_member(self, target: Self) -> bool {
        match self {
            Self::Owner => target != Self::Owner,
            Self::Admin => matches!(target, Self::Collaborator | Self::Viewer),
            _ => false,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectDto {
    id: Uuid,
    name: String,
    slug: String,
    owner_id: Uuid,
    role: ProjectRole,
    member_count: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct ProjectRow {
    id: Uuid,
    name: String,
    slug: String,
    owner_id: Uuid,
    role: String,
    member_count: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MemberDto {
    user_id: Uuid,
    email: String,
    display_name: String,
    role: ProjectRole,
    joined_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct MemberRow {
    user_id: Uuid,
    email: String,
    display_name: String,
    role: String,
    joined_at: DateTime<Utc>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AuditDto {
    id: Uuid,
    actor_id: Option<Uuid>,
    action: String,
    resource_type: String,
    resource_id: Option<Uuid>,
    metadata: serde_json::Value,
    created_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct AuditRow {
    id: Uuid,
    actor_id: Option<Uuid>,
    action: String,
    resource_type: String,
    resource_id: Option<Uuid>,
    metadata: serde_json::Value,
    created_at: DateTime<Utc>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateProjectRequest {
    name: String,
    slug: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateProjectRequest {
    name: Option<String>,
    slug: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AddMemberRequest {
    email: String,
    role: ProjectRole,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateMemberRequest {
    role: ProjectRole,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TransferOwnerRequest {
    user_id: Uuid,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/projects", get(list_projects).post(create_project))
        .route(
            "/projects/{project_id}",
            get(get_project).patch(update_project),
        )
        .route(
            "/projects/{project_id}/members",
            get(list_members).post(add_member),
        )
        .route(
            "/projects/{project_id}/members/{user_id}",
            axum::routing::patch(update_member).delete(remove_member),
        )
        .route(
            "/projects/{project_id}/transfer-owner",
            post(transfer_owner),
        )
        .route("/projects/{project_id}/audit", get(list_audit))
}

async fn list_projects(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
) -> Result<Json<Vec<ProjectDto>>, ApiError> {
    let rows: Vec<ProjectRow> = sqlx::query_as(
        r#"
        SELECT p.id, p.name, p.slug, p.owner_id, pm.role,
               (SELECT count(*) FROM project_members m WHERE m.project_id = p.id) AS member_count,
               p.created_at, p.updated_at
        FROM projects p
        JOIN project_members pm ON pm.project_id = p.id AND pm.user_id = $1
        ORDER BY p.created_at DESC
        LIMIT $2
        "#,
    )
    .bind(context.user.id)
    .bind(MAX_LISTED_PROJECTS)
    .fetch_all(&state.pool)
    .await
    .map_err(ApiError::database)?;

    rows.into_iter()
        .map(project_from_row)
        .collect::<Result<Vec<_>, _>>()
        .map(Json)
}

async fn create_project(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiJson(payload): ApiJson<CreateProjectRequest>,
) -> Result<Response, ApiError> {
    let name = validate_project_name(&payload.name)?;
    let slug = match payload.slug.as_deref() {
        Some(value) => validate_slug(value)?,
        None => slugify(&name)?,
    };

    let mut transaction = state.pool.begin().await.map_err(ApiError::database)?;
    let project_id: Uuid = sqlx::query_scalar(
        "INSERT INTO projects (name, slug, owner_id) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(&name)
    .bind(&slug)
    .bind(context.user.id)
    .fetch_one(&mut *transaction)
    .await
    .map_err(map_project_write_error)?;

    sqlx::query("INSERT INTO project_members (project_id, user_id, role) VALUES ($1, $2, 'owner')")
        .bind(project_id)
        .bind(context.user.id)
        .execute(&mut *transaction)
        .await
        .map_err(ApiError::database)?;

    crate::audit::record_in_tx(
        &mut transaction,
        Some(project_id),
        Some(context.user.id),
        "project.created",
        "project",
        Some(project_id),
        json!({ "slug": slug }),
    )
    .await
    .map_err(ApiError::database)?;

    let row = fetch_project_row(&mut transaction, context.user.id, project_id).await?;
    transaction.commit().await.map_err(ApiError::database)?;

    Ok((StatusCode::CREATED, Json(project_from_row(row)?)).into_response())
}

async fn get_project(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(project_id): ApiPath<Uuid>,
) -> Result<Json<ProjectDto>, ApiError> {
    let (project, _) = load_project(&state.pool, context.user.id, project_id).await?;
    Ok(Json(project))
}

async fn update_project(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(project_id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<UpdateProjectRequest>,
) -> Result<Json<ProjectDto>, ApiError> {
    if payload.name.is_none() && payload.slug.is_none() {
        return Err(ApiError::invalid(
            "no_changes",
            "Provide at least one project field to update.",
        ));
    }

    let name = payload
        .name
        .as_deref()
        .map(validate_project_name)
        .transpose()?;
    let slug = payload.slug.as_deref().map(validate_slug).transpose()?;

    let mut transaction = state.pool.begin().await.map_err(ApiError::database)?;
    let role = lock_actor_role(&mut transaction, project_id, context.user.id).await?;
    if !role.can_manage_project() {
        return Err(ApiError::forbidden(
            "forbidden",
            "You do not have permission to update this project.",
        ));
    }

    let current: (String, String) =
        sqlx::query_as("SELECT name, slug FROM projects WHERE id = $1 FOR UPDATE")
            .bind(project_id)
            .fetch_one(&mut *transaction)
            .await
            .map_err(ApiError::database)?;

    let changed_name = name.as_deref().is_some_and(|value| value != current.0);
    let changed_slug = slug.as_deref().is_some_and(|value| value != current.1);
    let next_name = name.unwrap_or(current.0);
    let next_slug = slug.unwrap_or(current.1);

    sqlx::query("UPDATE projects SET name = $2, slug = $3 WHERE id = $1")
        .bind(project_id)
        .bind(&next_name)
        .bind(&next_slug)
        .execute(&mut *transaction)
        .await
        .map_err(map_project_write_error)?;

    crate::audit::record_in_tx(
        &mut transaction,
        Some(project_id),
        Some(context.user.id),
        "project.updated",
        "project",
        Some(project_id),
        json!({
            "nameChanged": changed_name,
            "slugChanged": changed_slug,
        }),
    )
    .await
    .map_err(ApiError::database)?;

    let row = fetch_project_row(&mut transaction, context.user.id, project_id).await?;
    transaction.commit().await.map_err(ApiError::database)?;
    Ok(Json(project_from_row(row)?))
}

async fn list_members(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(project_id): ApiPath<Uuid>,
) -> Result<Json<Vec<MemberDto>>, ApiError> {
    let _ = load_project(&state.pool, context.user.id, project_id).await?;
    let rows: Vec<MemberRow> = sqlx::query_as(
        r#"
        SELECT u.id AS user_id, u.email, u.display_name, pm.role, pm.created_at AS joined_at
        FROM project_members pm
        JOIN users u ON u.id = pm.user_id
        WHERE pm.project_id = $1
        ORDER BY CASE pm.role
                   WHEN 'owner' THEN 0
                   WHEN 'admin' THEN 1
                   WHEN 'collaborator' THEN 2
                   ELSE 3
                 END,
                 u.display_name
        LIMIT $2
        "#,
    )
    .bind(project_id)
    .bind(MAX_LISTED_MEMBERS)
    .fetch_all(&state.pool)
    .await
    .map_err(ApiError::database)?;

    rows.into_iter()
        .map(member_from_row)
        .collect::<Result<Vec<_>, _>>()
        .map(Json)
}

async fn add_member(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(project_id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<AddMemberRequest>,
) -> Result<Response, ApiError> {
    if payload.role == ProjectRole::Owner {
        return Err(ApiError::invalid(
            "owner_role_not_assignable",
            "Ownership must be transferred explicitly, not assigned as a member role.",
        ));
    }

    let email = normalize_email(&payload.email)
        .ok_or_else(|| ApiError::invalid("invalid_email", "Enter a valid email address."))?;

    let mut transaction = state.pool.begin().await.map_err(ApiError::database)?;
    let actor_role = lock_actor_role(&mut transaction, project_id, context.user.id).await?;
    if !actor_role.can_manage_members() || !actor_role.can_manage_member(payload.role) {
        return Err(ApiError::forbidden(
            "forbidden",
            "You do not have permission to add this member.",
        ));
    }

    let target: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM users WHERE email = $1 AND status = 'active'")
            .bind(&email)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(ApiError::database)?;
    let Some(target_id) = target else {
        return Err(ApiError::not_found(
            "user_not_found",
            "No active account matches that email address.",
        ));
    };

    sqlx::query("INSERT INTO project_members (project_id, user_id, role) VALUES ($1, $2, $3)")
        .bind(project_id)
        .bind(target_id)
        .bind(role_to_db(payload.role))
        .execute(&mut *transaction)
        .await
        .map_err(|error| {
            if is_unique_violation(&error) {
                ApiError::conflict("already_member", "That user is already a project member.")
            } else {
                ApiError::database(error)
            }
        })?;

    crate::audit::record_in_tx(
        &mut transaction,
        Some(project_id),
        Some(context.user.id),
        "project.member_added",
        "project_member",
        Some(target_id),
        json!({ "memberUserId": target_id, "role": role_to_db(payload.role) }),
    )
    .await
    .map_err(ApiError::database)?;

    let member = fetch_member(&mut transaction, project_id, target_id).await?;
    transaction.commit().await.map_err(ApiError::database)?;
    Ok((StatusCode::CREATED, Json(member)).into_response())
}

async fn update_member(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath((project_id, user_id)): ApiPath<(Uuid, Uuid)>,
    ApiJson(payload): ApiJson<UpdateMemberRequest>,
) -> Result<Json<MemberDto>, ApiError> {
    if payload.role == ProjectRole::Owner {
        return Err(ApiError::invalid(
            "owner_role_not_assignable",
            "Ownership must be transferred explicitly, not assigned as a member role.",
        ));
    }

    let mut transaction = state.pool.begin().await.map_err(ApiError::database)?;
    let actor_role = lock_actor_role(&mut transaction, project_id, context.user.id).await?;
    if !actor_role.can_manage_members() {
        return Err(ApiError::forbidden(
            "forbidden",
            "You do not have permission to change project members.",
        ));
    }

    let target_role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM project_members WHERE project_id = $1 AND user_id = $2 FOR UPDATE",
    )
    .bind(project_id)
    .bind(user_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(ApiError::database)?;
    let Some(target_role) = target_role else {
        return Err(ApiError::not_found("member_not_found", "Member not found."));
    };
    let target_role = ProjectRole::from_db(&target_role)
        .ok_or_else(|| ApiError::internal("unsupported project role in database"))?;

    if target_role == ProjectRole::Owner || !actor_role.can_manage_member(target_role) {
        return Err(ApiError::forbidden(
            "owner_role_immutable",
            "The owner cannot be changed as a member; use ownership transfer.",
        ));
    }
    if !actor_role.can_manage_member(payload.role) {
        return Err(ApiError::forbidden(
            "forbidden",
            "You cannot assign that role.",
        ));
    }

    if target_role != payload.role {
        sqlx::query("UPDATE project_members SET role = $3 WHERE project_id = $1 AND user_id = $2")
            .bind(project_id)
            .bind(user_id)
            .bind(role_to_db(payload.role))
            .execute(&mut *transaction)
            .await
            .map_err(ApiError::database)?;

        crate::audit::record_in_tx(
            &mut transaction,
            Some(project_id),
            Some(context.user.id),
            "project.member_role_changed",
            "project_member",
            Some(user_id),
            json!({
                "memberUserId": user_id,
                "previousRole": role_to_db(target_role),
                "role": role_to_db(payload.role),
            }),
        )
        .await
        .map_err(ApiError::database)?;
    }

    let member = fetch_member(&mut transaction, project_id, user_id).await?;
    transaction.commit().await.map_err(ApiError::database)?;
    Ok(Json(member))
}

async fn remove_member(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath((project_id, user_id)): ApiPath<(Uuid, Uuid)>,
) -> Result<Response, ApiError> {
    let mut transaction = state.pool.begin().await.map_err(ApiError::database)?;
    let actor_role = lock_actor_role(&mut transaction, project_id, context.user.id).await?;
    if !actor_role.can_manage_members() {
        return Err(ApiError::forbidden(
            "forbidden",
            "You do not have permission to remove project members.",
        ));
    }

    let target_role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM project_members WHERE project_id = $1 AND user_id = $2 FOR UPDATE",
    )
    .bind(project_id)
    .bind(user_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(ApiError::database)?;
    let Some(target_role) = target_role else {
        return Err(ApiError::not_found("member_not_found", "Member not found."));
    };
    let target_role = ProjectRole::from_db(&target_role)
        .ok_or_else(|| ApiError::internal("unsupported project role in database"))?;

    if target_role == ProjectRole::Owner || !actor_role.can_manage_member(target_role) {
        return Err(ApiError::forbidden(
            "owner_role_immutable",
            "The owner cannot be removed; transfer ownership first.",
        ));
    }

    sqlx::query("DELETE FROM project_members WHERE project_id = $1 AND user_id = $2")
        .bind(project_id)
        .bind(user_id)
        .execute(&mut *transaction)
        .await
        .map_err(ApiError::database)?;

    crate::audit::record_in_tx(
        &mut transaction,
        Some(project_id),
        Some(context.user.id),
        "project.member_removed",
        "project_member",
        Some(user_id),
        json!({ "memberUserId": user_id, "previousRole": role_to_db(target_role) }),
    )
    .await
    .map_err(ApiError::database)?;

    transaction.commit().await.map_err(ApiError::database)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn transfer_owner(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(project_id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<TransferOwnerRequest>,
) -> Result<Json<ProjectDto>, ApiError> {
    if payload.user_id == context.user.id {
        return Err(ApiError::invalid(
            "same_owner",
            "Choose a different project member.",
        ));
    }

    let mut transaction = state.pool.begin().await.map_err(ApiError::database)?;
    let actor_role = lock_actor_role(&mut transaction, project_id, context.user.id).await?;
    if actor_role != ProjectRole::Owner {
        return Err(ApiError::forbidden(
            "forbidden",
            "Only the project owner can transfer ownership.",
        ));
    }

    let target_role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM project_members WHERE project_id = $1 AND user_id = $2 FOR UPDATE",
    )
    .bind(project_id)
    .bind(payload.user_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(ApiError::database)?;
    if target_role.is_none() {
        return Err(ApiError::invalid(
            "target_not_member",
            "The new owner must already be a project member.",
        ));
    }

    sqlx::query("UPDATE projects SET owner_id = $2 WHERE id = $1")
        .bind(project_id)
        .bind(payload.user_id)
        .execute(&mut *transaction)
        .await
        .map_err(ApiError::database)?;
    sqlx::query("UPDATE project_members SET role = 'admin' WHERE project_id = $1 AND user_id = $2")
        .bind(project_id)
        .bind(context.user.id)
        .execute(&mut *transaction)
        .await
        .map_err(ApiError::database)?;
    sqlx::query("UPDATE project_members SET role = 'owner' WHERE project_id = $1 AND user_id = $2")
        .bind(project_id)
        .bind(payload.user_id)
        .execute(&mut *transaction)
        .await
        .map_err(ApiError::database)?;

    crate::audit::record_in_tx(
        &mut transaction,
        Some(project_id),
        Some(context.user.id),
        "project.owner_transferred",
        "project",
        Some(project_id),
        json!({
            "previousOwnerId": context.user.id,
            "newOwnerId": payload.user_id,
        }),
    )
    .await
    .map_err(ApiError::database)?;

    let row = fetch_project_row(&mut transaction, context.user.id, project_id).await?;
    transaction.commit().await.map_err(ApiError::database)?;
    Ok(Json(project_from_row(row)?))
}

async fn list_audit(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(project_id): ApiPath<Uuid>,
    ApiQuery(query): ApiQuery<AuditQuery>,
) -> Result<Json<Vec<AuditDto>>, ApiError> {
    let (_, role) = load_project(&state.pool, context.user.id, project_id).await?;
    if !role.can_view_audit() {
        return Err(ApiError::forbidden(
            "forbidden",
            "You do not have permission to view the project audit trail.",
        ));
    }
    let limit = query.limit.unwrap_or(50).clamp(1, MAX_AUDIT_LIMIT);

    let rows: Vec<AuditRow> = sqlx::query_as(
        r#"
        SELECT id, actor_id, action, resource_type, resource_id, metadata, created_at
        FROM audit_log
        WHERE project_id = $1
        ORDER BY created_at DESC
        LIMIT $2
        "#,
    )
    .bind(project_id)
    .bind(limit)
    .fetch_all(&state.pool)
    .await
    .map_err(ApiError::database)?;

    Ok(Json(
        rows.into_iter()
            .map(|row| AuditDto {
                id: row.id,
                actor_id: row.actor_id,
                action: row.action,
                resource_type: row.resource_type,
                resource_id: row.resource_id,
                metadata: row.metadata,
                created_at: row.created_at,
            })
            .collect(),
    ))
}

pub(crate) async fn load_project_role(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    project_id: Uuid,
) -> Result<ProjectRole, ApiError> {
    let role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM project_members WHERE project_id = $1 AND user_id = $2",
    )
    .bind(project_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::database)?;

    let Some(role) = role else {
        return Err(ApiError::not_found(
            "project_not_found",
            "Project not found.",
        ));
    };
    ProjectRole::from_db(&role)
        .ok_or_else(|| ApiError::internal("unsupported project role in database"))
}

async fn load_project(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    project_id: Uuid,
) -> Result<(ProjectDto, ProjectRole), ApiError> {
    let row: Option<ProjectRow> = sqlx::query_as(
        r#"
        SELECT p.id, p.name, p.slug, p.owner_id, pm.role,
               (SELECT count(*) FROM project_members m WHERE m.project_id = p.id) AS member_count,
               p.created_at, p.updated_at
        FROM projects p
        JOIN project_members pm ON pm.project_id = p.id AND pm.user_id = $2
        WHERE p.id = $1
        "#,
    )
    .bind(project_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::database)?;

    let Some(row) = row else {
        // Non-members receive 404 so project existence is not disclosed.
        return Err(ApiError::not_found(
            "project_not_found",
            "Project not found.",
        ));
    };
    let role = ProjectRole::from_db(&row.role)
        .ok_or_else(|| ApiError::internal("unsupported project role in database"))?;
    Ok((project_from_row(row)?, role))
}

async fn lock_actor_role(
    transaction: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    user_id: Uuid,
) -> Result<ProjectRole, ApiError> {
    let role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM project_members
         WHERE project_id = $1 AND user_id = $2
         FOR UPDATE",
    )
    .bind(project_id)
    .bind(user_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(ApiError::database)?;

    match role {
        Some(role) => ProjectRole::from_db(&role)
            .ok_or_else(|| ApiError::internal("unsupported project role in database")),
        None => Err(ApiError::not_found(
            "project_not_found",
            "Project not found.",
        )),
    }
}

async fn fetch_project_row(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    project_id: Uuid,
) -> Result<ProjectRow, ApiError> {
    sqlx::query_as(
        r#"
        SELECT p.id, p.name, p.slug, p.owner_id, pm.role,
               (SELECT count(*) FROM project_members m WHERE m.project_id = p.id) AS member_count,
               p.created_at, p.updated_at
        FROM projects p
        JOIN project_members pm ON pm.project_id = p.id AND pm.user_id = $2
        WHERE p.id = $1
        "#,
    )
    .bind(project_id)
    .bind(user_id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(ApiError::database)
}

async fn fetch_member(
    transaction: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    user_id: Uuid,
) -> Result<MemberDto, ApiError> {
    let row: MemberRow = sqlx::query_as(
        r#"
        SELECT u.id AS user_id, u.email, u.display_name, pm.role, pm.created_at AS joined_at
        FROM project_members pm
        JOIN users u ON u.id = pm.user_id
        WHERE pm.project_id = $1 AND pm.user_id = $2
        "#,
    )
    .bind(project_id)
    .bind(user_id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(ApiError::database)?;
    member_from_row(row)
}

fn project_from_row(row: ProjectRow) -> Result<ProjectDto, ApiError> {
    Ok(ProjectDto {
        id: row.id,
        name: row.name,
        slug: row.slug,
        owner_id: row.owner_id,
        role: ProjectRole::from_db(&row.role)
            .ok_or_else(|| ApiError::internal("unsupported project role in database"))?,
        member_count: row.member_count,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

fn member_from_row(row: MemberRow) -> Result<MemberDto, ApiError> {
    Ok(MemberDto {
        user_id: row.user_id,
        email: row.email,
        display_name: row.display_name,
        role: ProjectRole::from_db(&row.role)
            .ok_or_else(|| ApiError::internal("unsupported project role in database"))?,
        joined_at: row.joined_at,
    })
}

fn role_to_db(role: ProjectRole) -> &'static str {
    match role {
        ProjectRole::Owner => "owner",
        ProjectRole::Admin => "admin",
        ProjectRole::Collaborator => "collaborator",
        ProjectRole::Viewer => "viewer",
    }
}

fn validate_project_name(value: &str) -> Result<String, ApiError> {
    let name = value.trim();
    if !(1..=120).contains(&name.chars().count()) || name.chars().any(char::is_control) {
        return Err(ApiError::invalid(
            "invalid_project_name",
            "Project name must be between 1 and 120 characters.",
        ));
    }
    Ok(name.to_owned())
}

fn validate_slug(value: &str) -> Result<String, ApiError> {
    let slug = value.trim().to_ascii_lowercase();
    let valid = (2..=63).contains(&slug.len())
        && slug
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && slug
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if !valid {
        return Err(ApiError::invalid(
            "invalid_project_slug",
            "Project slug must be 2-63 lowercase letters, digits, or hyphens, and cannot start or end with a hyphen.",
        ));
    }
    Ok(slug)
}

fn slugify(name: &str) -> Result<String, ApiError> {
    let mut slug = String::with_capacity(name.len());
    let mut last_was_hyphen = false;
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
            last_was_hyphen = false;
        } else if !last_was_hyphen && !slug.is_empty() {
            slug.push('-');
            last_was_hyphen = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    slug.truncate(63);
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.len() < 2 {
        return Err(ApiError::invalid(
            "invalid_project_slug",
            "Could not derive a slug from the project name; provide a slug.",
        ));
    }
    Ok(slug)
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    matches!(
        error,
        sqlx::Error::Database(database_error)
            if database_error.code().as_deref() == Some("23505")
    )
}

fn map_project_write_error(error: sqlx::Error) -> ApiError {
    if is_unique_violation(&error) {
        ApiError::conflict(
            "project_slug_taken",
            "A project with this slug already exists.",
        )
    } else {
        ApiError::database(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_matrix_is_least_privilege() {
        assert!(ProjectRole::Owner.can_manage_project());
        assert!(ProjectRole::Admin.can_manage_project());
        assert!(!ProjectRole::Collaborator.can_manage_project());
        assert!(!ProjectRole::Viewer.can_manage_project());
        assert!(ProjectRole::Owner.can_create_jobs());
        assert!(ProjectRole::Admin.can_create_jobs());
        assert!(ProjectRole::Collaborator.can_create_jobs());
        assert!(!ProjectRole::Viewer.can_create_jobs());

        assert!(ProjectRole::Owner.can_manage_member(ProjectRole::Admin));
        assert!(ProjectRole::Owner.can_manage_member(ProjectRole::Viewer));
        assert!(!ProjectRole::Admin.can_manage_member(ProjectRole::Admin));
        assert!(ProjectRole::Admin.can_manage_member(ProjectRole::Collaborator));
        assert!(!ProjectRole::Viewer.can_manage_member(ProjectRole::Viewer));
        assert!(!ProjectRole::Owner.can_manage_member(ProjectRole::Owner));
    }

    #[test]
    fn validates_and_derives_slugs() {
        assert_eq!(slugify("Payments API").expect("slug"), "payments-api");
        assert_eq!(slugify("  A  B  ").expect("slug"), "a-b");
        assert!(slugify("!!").is_err());
        assert!(validate_slug("payments-api").is_ok());
        assert!(validate_slug("-bad").is_err());
        assert!(validate_slug("bad-").is_err());
        assert!(validate_slug("Has Spaces").is_err());
    }

    #[test]
    fn rejects_control_characters_in_names() {
        assert!(validate_project_name("ok").is_ok());
        assert!(validate_project_name("bad\nname").is_err());
        assert!(validate_project_name("   ").is_err());
    }
}

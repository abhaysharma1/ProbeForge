//! Project-scoped repository registry.
//!
//! `git_sources` was previously only written as a side effect of job creation, so
//! a project had no way to list or remove a connected repository. These endpoints
//! make it a first-class resource:
//!
//! - list active repositories for a project (any member);
//! - register a repository without starting an analysis (non-viewer);
//! - remove (archive) a repository (owner or admin).
//!
//! Removal archives the row rather than deleting it. Job history (plans,
//! approvals, runs, evidence) and `job_events` are append-only and protected by
//! delete-rejecting triggers, so a hard delete would either be rejected or destroy
//! immutable records. Archiving hides the repository from the active list and
//! blocks new analysis while preserving history; reconnecting the same URL
//! reactivates it.

use axum::{
    Extension, Json, Router,
    extract::State,
    http::StatusCode,
    routing::{delete, get},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{FromRow, Postgres, Transaction};
use url::Url;
use uuid::Uuid;

use crate::{
    AppState, audit,
    auth::AuthContext,
    error::{ApiError, ApiJson, ApiPath},
    jobs::provider_for,
    projects::load_project_role,
};

const MAX_LISTED_REPOSITORIES: i64 = 100;
const MAX_DISPLAY_NAME_LEN: usize = 120;
const MAX_URL_LEN: usize = 2048;
const MIN_URL_LEN: usize = 8;
const MAX_REFERENCE_LEN: usize = 200;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/projects/{project_id}/repositories",
            get(list_repositories).post(register_repository),
        )
        .route(
            "/projects/{project_id}/repositories/{repository_id}",
            delete(remove_repository),
        )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RegisterRepositoryRequest {
    repository_url: String,
    reference: String,
    #[serde(default)]
    display_name: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RepositoryDto {
    id: Uuid,
    project_id: Uuid,
    provider: String,
    repository_url: String,
    default_reference: String,
    display_name: Option<String>,
    job_count: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct RepositoryRow {
    id: Uuid,
    project_id: Uuid,
    provider: String,
    repository_url: String,
    default_reference: String,
    display_name: Option<String>,
    job_count: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

const REPOSITORY_COLUMNS: &str = r#"
    g.id, g.project_id, g.provider, g.normalized_url AS repository_url,
    g.default_reference, g.display_name,
    (SELECT count(*) FROM jobs j WHERE j.git_source_id = g.id) AS job_count,
    g.created_at, g.updated_at
"#;

async fn list_repositories(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(project_id): ApiPath<Uuid>,
) -> Result<Json<Vec<RepositoryDto>>, ApiError> {
    load_project_role(&state.pool, context.user.id, project_id).await?;
    let query = format!(
        "SELECT {REPOSITORY_COLUMNS} FROM git_sources g \
         WHERE g.project_id = $1 AND g.archived_at IS NULL \
         ORDER BY g.created_at DESC LIMIT $2"
    );
    let rows: Vec<RepositoryRow> = sqlx::query_as(sqlx::AssertSqlSafe(query))
        .bind(project_id)
        .bind(MAX_LISTED_REPOSITORIES)
        .fetch_all(&state.pool)
        .await
        .map_err(ApiError::database)?;
    Ok(Json(rows.into_iter().map(repository_dto).collect()))
}

async fn register_repository(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(project_id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<RegisterRepositoryRequest>,
) -> Result<(StatusCode, Json<RepositoryDto>), ApiError> {
    let role = load_project_role(&state.pool, context.user.id, project_id).await?;
    if !role.can_create_jobs() {
        tracing::warn!(
            event = "repository.register_forbidden",
            project_id = %project_id,
            user_id = %context.user.id,
            "repository registration denied: insufficient project role"
        );
        return Err(ApiError::forbidden(
            "forbidden",
            "You do not have permission to add repositories to this project.",
        ));
    }

    let repository_url = normalize_repository_url(&payload.repository_url)?;
    let provider = provider_for(&repository_url)?;
    let reference = normalize_reference(&payload.reference)?;
    let display_name = normalize_display_name(payload.display_name.as_deref())?;

    let mut transaction = state.pool.begin().await.map_err(ApiError::database)?;
    let repository_id = upsert_repository(
        &mut transaction,
        project_id,
        provider,
        &repository_url,
        &reference,
        display_name.as_deref(),
    )
    .await?;

    audit::record_in_tx(
        &mut transaction,
        Some(project_id),
        Some(context.user.id),
        "repository.registered",
        "repository",
        Some(repository_id),
        json!({ "provider": provider, "repositoryUrl": repository_url }),
    )
    .await
    .map_err(ApiError::database)?;

    let row = load_repository_row(&mut transaction, project_id, repository_id).await?;
    transaction.commit().await.map_err(ApiError::database)?;
    tracing::info!(
        event = "repository.registered",
        repository_id = %repository_id,
        project_id = %project_id,
        user_id = %context.user.id,
        provider,
        "repository registered"
    );
    Ok((StatusCode::CREATED, Json(repository_dto(row))))
}

async fn remove_repository(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath((project_id, repository_id)): ApiPath<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let role = load_project_role(&state.pool, context.user.id, project_id).await?;
    if !role.can_manage_repositories() {
        tracing::warn!(
            event = "repository.remove_forbidden",
            repository_id = %repository_id,
            project_id = %project_id,
            user_id = %context.user.id,
            role = ?role,
            "repository removal denied: insufficient project role"
        );
        return Err(ApiError::forbidden(
            "forbidden",
            "Only an owner or admin can remove a repository from this project.",
        ));
    }

    let mut transaction = state.pool.begin().await.map_err(ApiError::database)?;
    let existing: Option<(Uuid, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT project_id, archived_at FROM git_sources WHERE id = $1 AND project_id = $2 FOR UPDATE",
    )
    .bind(repository_id)
    .bind(project_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(ApiError::database)?;

    let Some((owner_project_id, archived_at)) = existing else {
        return Err(ApiError::not_found(
            "repository_not_found",
            "Repository not found.",
        ));
    };

    // Idempotent: removing an already-archived repository succeeds without a new
    // audit row or event.
    if archived_at.is_some() {
        transaction.commit().await.map_err(ApiError::database)?;
        return Ok(StatusCode::NO_CONTENT);
    }

    let in_flight: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM jobs \
         WHERE git_source_id = $1 AND status IN ('queued', 'analyzing', 'planning') \
         LIMIT 1",
    )
    .bind(repository_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(ApiError::database)?;
    if in_flight.is_some() {
        return Err(ApiError::conflict(
            "repository_busy",
            "Wait for the active analysis on this repository to finish before removing it.",
        ));
    }

    sqlx::query("UPDATE git_sources SET archived_at = now() WHERE id = $1 AND archived_at IS NULL")
        .bind(repository_id)
        .execute(&mut *transaction)
        .await
        .map_err(ApiError::database)?;

    audit::record_in_tx(
        &mut transaction,
        Some(owner_project_id),
        Some(context.user.id),
        "repository.removed",
        "repository",
        Some(repository_id),
        json!({ "projectId": owner_project_id }),
    )
    .await
    .map_err(ApiError::database)?;

    transaction.commit().await.map_err(ApiError::database)?;
    tracing::info!(
        event = "repository.removed",
        repository_id = %repository_id,
        project_id = %owner_project_id,
        user_id = %context.user.id,
        "repository archived"
    );
    Ok(StatusCode::NO_CONTENT)
}

async fn upsert_repository(
    transaction: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    provider: &'static str,
    repository_url: &str,
    default_reference: &str,
    display_name: Option<&str>,
) -> Result<Uuid, ApiError> {
    let repository_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO git_sources (
            project_id, provider, normalized_url, default_reference, display_name
        )
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (project_id, normalized_url)
        DO UPDATE SET
            default_reference = EXCLUDED.default_reference,
            display_name = COALESCE(EXCLUDED.display_name, git_sources.display_name),
            archived_at = NULL,
            updated_at = now()
        RETURNING id
        "#,
    )
    .bind(project_id)
    .bind(provider)
    .bind(repository_url)
    .bind(default_reference)
    .bind(display_name)
    .fetch_one(&mut **transaction)
    .await
    .map_err(ApiError::database)?;
    Ok(repository_id)
}

async fn load_repository_row(
    transaction: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    repository_id: Uuid,
) -> Result<RepositoryRow, ApiError> {
    let query = format!(
        "SELECT {REPOSITORY_COLUMNS} FROM git_sources g WHERE g.id = $1 AND g.project_id = $2"
    );
    let row: Option<RepositoryRow> = sqlx::query_as(sqlx::AssertSqlSafe(query))
        .bind(repository_id)
        .bind(project_id)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(ApiError::database)?;
    row.ok_or_else(|| ApiError::not_found("repository_not_found", "Repository not found."))
}

fn repository_dto(row: RepositoryRow) -> RepositoryDto {
    RepositoryDto {
        id: row.id,
        project_id: row.project_id,
        provider: row.provider,
        repository_url: row.repository_url,
        default_reference: row.default_reference,
        display_name: row.display_name,
        job_count: row.job_count,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

/// Accept only `https` or `ssh` Git URLs without embedded credentials. The SSH
/// username, when present, must be `git`; a URL query or fragment is rejected so
/// the stored value is a plain repository locator.
fn normalize_repository_url(raw: &str) -> Result<String, ApiError> {
    let value = raw.trim();
    if value.len() < MIN_URL_LEN || value.len() > MAX_URL_LEN {
        return Err(invalid_url());
    }
    let parsed = Url::parse(value).map_err(|_| invalid_url())?;
    match parsed.scheme() {
        "https" => {
            if parsed.password().is_some() || parsed.host_str().is_none() {
                return Err(invalid_url());
            }
            if !parsed.username().is_empty() {
                return Err(ApiError::invalid(
                    "invalid_repository_url",
                    "Repository URL must not include a username.",
                ));
            }
        }
        "ssh" => {
            if parsed.password().is_some() || parsed.host_str().is_none() {
                return Err(invalid_url());
            }
            let username = parsed.username();
            if !username.is_empty() && username != "git" {
                return Err(ApiError::invalid(
                    "invalid_repository_url",
                    "SSH repository URL must use the `git` username.",
                ));
            }
        }
        _ => return Err(invalid_url()),
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(invalid_url());
    }
    Ok(value.to_owned())
}

/// Mirror the database's `jobs_source_reference_safe` rule so the API rejects a
/// bad reference with a clear error instead of a constraint failure.
fn normalize_reference(raw: &str) -> Result<String, ApiError> {
    let value = raw.trim();
    let valid = !value.is_empty()
        && value.len() <= MAX_REFERENCE_LEN
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-'))
        && !value.starts_with('-')
        && !value.contains("..");
    if !valid {
        return Err(ApiError::invalid(
            "invalid_reference",
            "Reference must be a branch, tag, or commit SHA without spaces or path traversal.",
        ));
    }
    Ok(value.to_owned())
}

fn normalize_display_name(raw: Option<&str>) -> Result<Option<String>, ApiError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let value = raw.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.chars().count() > MAX_DISPLAY_NAME_LEN || value.chars().any(char::is_control) {
        return Err(ApiError::invalid(
            "invalid_display_name",
            "Repository name must be 1-120 printable characters.",
        ));
    }
    Ok(Some(value.to_owned()))
}

fn invalid_url() -> ApiError {
    ApiError::invalid(
        "invalid_repository_url",
        "Repository URL must be a valid https or ssh Git URL without embedded credentials.",
    )
}

#[cfg(test)]
mod tests {
    use super::{normalize_display_name, normalize_reference, normalize_repository_url};

    #[test]
    fn accepts_public_and_ssh_repository_urls() {
        assert_eq!(
            normalize_repository_url("https://github.com/acme/app.git").expect("https"),
            "https://github.com/acme/app.git"
        );
        assert_eq!(
            normalize_repository_url("ssh://git@gitlab.com/acme/app.git").expect("ssh"),
            "ssh://git@gitlab.com/acme/app.git"
        );
    }

    #[test]
    fn rejects_credentials_and_unsafe_schemes() {
        assert!(normalize_repository_url("http://example.test/acme/app.git").is_err());
        assert!(normalize_repository_url("https://user:pass@example.test/acme/app.git").is_err());
        assert!(normalize_repository_url("https://user@example.test/acme/app.git").is_err());
        assert!(normalize_repository_url("ssh://root@example.test/acme/app.git").is_err());
        assert!(normalize_repository_url("https://example.test/acme/app.git?x=1").is_err());
    }

    #[test]
    fn validates_references() {
        assert_eq!(normalize_reference(" main ").expect("ok"), "main");
        assert!(normalize_reference("-flag").is_err());
        assert!(normalize_reference("feature/../etc").is_err());
        assert!(normalize_reference("has space").is_err());
        assert!(normalize_reference("").is_err());
    }

    #[test]
    fn normalizes_display_names() {
        assert_eq!(normalize_display_name(None).expect("none"), None);
        assert_eq!(normalize_display_name(Some("  ")).expect("blank"), None);
        assert_eq!(
            normalize_display_name(Some("  Payments API  ")).expect("name"),
            Some("Payments API".to_owned())
        );
        assert!(normalize_display_name(Some(&"x".repeat(121))).is_err());
    }
}

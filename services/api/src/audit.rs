use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

/// Append an audit row inside the caller's transaction. Audit rows are
/// immutable at the database layer; this helper never updates or deletes them.
pub async fn record_in_tx(
    transaction: &mut Transaction<'_, Postgres>,
    project_id: Option<Uuid>,
    actor_id: Option<Uuid>,
    action: &str,
    resource_type: &str,
    resource_id: Option<Uuid>,
    metadata: Value,
) -> Result<(), sqlx::Error> {
    let metadata = if metadata.is_object() {
        metadata
    } else {
        json!({ "redacted": true })
    };

    sqlx::query(
        r#"
        INSERT INTO audit_log (project_id, actor_id, action, resource_type, resource_id, metadata)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(project_id)
    .bind(actor_id)
    .bind(action)
    .bind(resource_type)
    .bind(resource_id)
    .bind(metadata)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

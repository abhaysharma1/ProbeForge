//! Apply versioned SQL migrations. Run with `DATABASE_URL` set.
//!
//! The connection string is never logged. Migrations are forward-only; see
//! `docs/DATABASE.md` for the rollback and recovery policy.

use std::env;

use sqlx::{migrate::Migrator, postgres::PgPoolOptions};
use tracing_subscriber::EnvFilter;

/// Migrations compiled into this binary. The API and scheduler readiness gates
/// embed the same set, so the runner and `/readyz` cannot disagree about which
/// versions the deployment expects, and a packaged image does not need the
/// `migrations/` tree at runtime. Applying a newer migration therefore requires
/// running the migrate binary built from that newer commit.
static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let database_url = env::var("DATABASE_URL")
        .map_err(|_| "DATABASE_URL must be set before running migrations")?;
    if !database_url.starts_with("postgres://") && !database_url.starts_with("postgresql://") {
        return Err("DATABASE_URL must use the postgres:// or postgresql:// scheme".into());
    }

    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .map_err(|_| "database connection failed; verify DATABASE_URL and server availability")?;

    MIGRATOR.run(&pool).await?;

    tracing::info!(
        migrations = MIGRATOR.migrations.len(),
        "database migrations applied"
    );
    Ok(())
}

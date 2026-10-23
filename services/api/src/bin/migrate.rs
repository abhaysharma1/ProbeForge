//! Apply versioned SQL migrations. Run with `DATABASE_URL` set.
//!
//! The connection string is never logged. Migrations are forward-only; see
//! `docs/DATABASE.md` for the rollback and recovery policy.

use std::{
    env,
    error::Error,
    path::{Path, PathBuf},
};

use sqlx::{migrate::Migrator, postgres::PgPoolOptions};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
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

    let migrations_dir = env::var_os("MIGRATIONS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations"));

    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .map_err(|_| "database connection failed; verify DATABASE_URL and server availability")?;

    let migrator = Migrator::new(migrations_dir).await?;
    migrator.run(&pool).await?;

    tracing::info!("database migrations applied");
    Ok(())
}

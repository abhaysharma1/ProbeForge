//! Operator-invoked dispatch loop. It claims and executes queued runs until the
//! queue is idle, or continuously when `--once` is omitted. Network traffic goes
//! only to authorized targets, and loopback/private targets stay blocked unless
//! `PROBE_FORGE_ALLOW_PRIVATE_TARGETS=true` is set for a secured local mock.

use std::time::Duration;

use probe_forge_execution_worker::{DispatchConfig, SERVICE_NAME, dispatch_once};
use sqlx::postgres::PgPoolOptions;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    probe_forge_observability::init_tracing(SERVICE_NAME);

    let once = std::env::args().any(|argument| argument == "--once");
    let database_url =
        std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL must be set for dispatch")?;
    let worker_id = std::env::var("PROBE_FORGE_EXECUTION_WORKER_ID")
        .unwrap_or_else(|_| "execution-worker-1".to_owned());
    let allow_private = matches!(
        std::env::var("PROBE_FORGE_ALLOW_PRIVATE_TARGETS").as_deref(),
        Ok("true") | Ok("1")
    );

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&database_url)
        .await?;

    let mut config = DispatchConfig::new(worker_id);
    config.allow_private_targets = allow_private;

    loop {
        match dispatch_once(&pool, &config).await {
            Ok(Some(run)) => {
                tracing::info!(
                    run_id = %run.run_id,
                    run_type = %run.run_type,
                    status = run.status,
                    "dispatched run"
                );
            }
            Ok(None) => {
                if once {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(error) => {
                tracing::warn!(error = %error, "dispatch attempt failed");
                if once {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
    Ok(())
}

use probe_forge_api::{Config, SERVICE_NAME, serve};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    probe_forge_observability::init_tracing(SERVICE_NAME);

    let config = Config::from_env()?;
    serve(config).await?;
    Ok(())
}

use ssma_rust::gateway;
use ssma_rust::config::Config;
#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let config = Config::from_env();
    tracing::info!(subprotocol = %config.subprotocol, "ssma-rust boot");

    // Validate configuration at startup
    if let Err(error) = config.validate() {
        tracing::error!(%error, "ssma-rust configuration validation failed");
        eprintln!("Configuration error: {}", error);
        std::process::exit(1);
    }
    tracing::info!("ssma-rust configuration validated");

    if let Err(error) = gateway::run(config).await {
        tracing::error!(%error, "ssma-rust failed");
    }
}

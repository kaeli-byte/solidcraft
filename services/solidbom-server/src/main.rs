//! The SolidBOM development service.
//!
//! Configure it with `SOLIDBOM_DATABASE_URL`, `SOLIDBOM_TOKEN` (the one development bearer
//! identity), and optionally `SOLIDBOM_BIND` (loopback by default). The service applies the
//! migrations on start.

use solidbom_server::Store;
use solidbom_server::api::{ApiState, router};

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("solidbom-server: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    let url = required("SOLIDBOM_DATABASE_URL")?;
    let token = required("SOLIDBOM_TOKEN")?;
    if token.len() < 16 {
        return Err(std::io::Error::other("SOLIDBOM_TOKEN must be at least 16 characters").into());
    }
    let bind = std::env::var("SOLIDBOM_BIND").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    let store = Store::connect(&url).await?;
    let app = router(ApiState::new(store, token));
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    println!("solidbom-server: listening on {bind}");
    axum::serve(listener, app).await?;
    Ok(())
}

fn required(name: &str) -> Result<String, Box<dyn std::error::Error>> {
    std::env::var(name).map_err(|_| std::io::Error::other(format!("set {name}")).into())
}

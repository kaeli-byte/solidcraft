//! The SolidBOM development service.
//!
//! Configure it with `SOLIDBOM_DATABASE_URL`, `SOLIDBOM_TOKEN` (the combined legacy development
//! identity), and optionally `SOLIDBOM_ENGINEER_TOKEN`, `SOLIDBOM_ADMIN_TOKEN`, `SOLIDBOM_BIND`
//! (loopback by default), `SOLIDBOM_MAX_UPLOAD_BYTES` and `SOLIDBOM_REQUEST_TIMEOUT_MS`. Each token
//! is at least 16 characters and all tokens must be distinct; a bad configuration stops the
//! service before it listens. The service applies its migrations on start; the second migration
//! seeds the explicit reference-data test fixtures. See `README.md` in this directory for the
//! start, migration, seed and isolated test-database commands.

use std::time::Duration;

use solidbom_server::Store;
use solidbom_server::api::{ApiState, DEFAULT_REQUEST_TIMEOUT, MAX_UPLOAD_BYTES, router};
use solidbom_server::auth::IdentityTable;

/// The default bind: the prototype is a local development service. A remote development endpoint
/// needs a TLS-terminating proxy; the service itself speaks plain HTTP on loopback.
const DEFAULT_BIND: &str = "127.0.0.1:8080";

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
    // The legacy combined identity keeps an existing single-token setup working. The two optional
    // tokens add an engineer-only and an administrator-only identity; all tokens must be distinct.
    let mut identities = IdentityTable::development(&required("SOLIDBOM_TOKEN")?);
    if let Some(engineer) = optional("SOLIDBOM_ENGINEER_TOKEN") {
        identities = identities.with_engineer_token(&engineer);
    }
    if let Some(admin) = optional("SOLIDBOM_ADMIN_TOKEN") {
        identities = identities.with_administrator_token(&admin);
    }
    identities.validate().map_err(std::io::Error::other)?;
    let bind = std::env::var("SOLIDBOM_BIND").unwrap_or_else(|_| DEFAULT_BIND.to_string());
    let max_upload = optional_positive("SOLIDBOM_MAX_UPLOAD_BYTES", MAX_UPLOAD_BYTES)?;
    let request_timeout = optional_timeout("SOLIDBOM_REQUEST_TIMEOUT_MS", DEFAULT_REQUEST_TIMEOUT)?;
    let delay = optional_delay("SOLIDBOM_DELAY_MS")?;
    let store = Store::connect(&url).await?;
    let app =
        router(ApiState::with_identities(store, identities).with_max_upload(max_upload).with_request_timeout(request_timeout).with_delay(delay));
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    println!("solidbom-server: listening on {bind}");
    axum::serve(listener, app).with_graceful_shutdown(shutdown()).await?;
    Ok(())
}

/// Stop on Ctrl-C, and on SIGTERM on Unix, so a restarted desktop finds the same committed state.
async fn shutdown() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        if let Ok(mut terminate) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = terminate.recv() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

fn required(name: &str) -> Result<String, Box<dyn std::error::Error>> {
    std::env::var(name).map_err(|_| std::io::Error::other(format!("set {name}")).into())
}

fn optional_positive(name: &str, default: usize) -> Result<usize, Box<dyn std::error::Error>> {
    let Some(text) = optional(name) else { return Ok(default) };
    let value: usize = text.parse().map_err(|_| std::io::Error::other(format!("{name} must be a positive integer")))?;
    if value == 0 {
        return Err(std::io::Error::other(format!("{name} must be positive")).into());
    }
    Ok(value)
}

fn optional_timeout(name: &str, default: Duration) -> Result<Duration, Box<dyn std::error::Error>> {
    let Some(text) = optional(name) else { return Ok(default) };
    let millis: u64 = text.parse().map_err(|_| std::io::Error::other(format!("{name} must be a positive integer of milliseconds")))?;
    if millis == 0 {
        return Err(std::io::Error::other(format!("{name} must be positive")).into());
    }
    Ok(Duration::from_millis(millis))
}

/// A controlled delay for responsiveness measurements only; zero is the normal value.
fn optional_delay(name: &str) -> Result<Duration, Box<dyn std::error::Error>> {
    let Some(text) = optional(name) else { return Ok(Duration::ZERO) };
    let millis: u64 = text.parse().map_err(|_| std::io::Error::other(format!("{name} must be a non-negative integer of milliseconds")))?;
    Ok(Duration::from_millis(millis))
}

fn optional(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(text) if !text.trim().is_empty() => Some(text),
        _ => None,
    }
}

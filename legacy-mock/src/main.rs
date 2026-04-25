use anyhow::{Context, Result};
use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Router,
};
use reqwest::Client;
use std::sync::Once;

const DEFAULT_LISTEN_ADDRESS: &str = "0.0.0.0:8081";

#[derive(Clone)]
struct AppState {
    faas_url: String,
    http_client: Client,
}

fn ensure_rustls_crypto_provider() {
    static INIT: Once = Once::new();

    INIT.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

fn build_http_client() -> Client {
    ensure_rustls_crypto_provider();
    Client::new()
}

#[tokio::main]
async fn main() -> Result<()> {
    let listen_address =
        std::env::var("LEGACY_MOCK_ADDRESS").unwrap_or_else(|_| DEFAULT_LISTEN_ADDRESS.to_owned());
    let faas_url = std::env::var("FAAS_URL").context("FAAS_URL must be set")?;
    let listener = tokio::net::TcpListener::bind(&listen_address)
        .await
        .with_context(|| format!("failed to bind legacy-mock listener on {listen_address}"))?;

    let app = Router::new()
        .route("/ping", get(ping))
        .route("/call-faas", post(call_faas))
        .with_state(AppState {
            faas_url,
            http_client: build_http_client(),
        });

    axum::serve(listener, app)
        .await
        .context("legacy-mock server exited unexpectedly")
}

async fn ping() -> &'static str {
    "legacy_ok"
}

async fn call_faas(State(state): State<AppState>) -> Result<String, (StatusCode, String)> {
    let response = state
        .http_client
        .get(&state.faas_url)
        .send()
        .await
        .map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("failed to call FaaS URL `{}`: {error}", state.faas_url),
            )
        })?
        .error_for_status()
        .map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("FaaS URL `{}` returned an error: {error}", state.faas_url),
            )
        })?;

    response.text().await.map_err(|error| {
        (
            StatusCode::BAD_GATEWAY,
            format!(
                "failed to read FaaS response body from `{}`: {error}",
                state.faas_url
            ),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::build_http_client;

    #[test]
    fn reqwest_client_initializes_with_default_tls_provider() {
        let _client = build_http_client();
    }
}

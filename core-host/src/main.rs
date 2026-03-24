use anyhow::{anyhow, Context, Result};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::post,
    Router,
};
use std::path::PathBuf;
use wasmtime::{Engine, Linker, Module, Store, TypedFunc};
use wasmtime_wasi::{
    p1::{self, WasiP1Ctx},
    p2::pipe::{MemoryInputPipe, MemoryOutputPipe},
    WasiCtxBuilder,
};

const HOST_ADDRESS: &str = "0.0.0.0:8080";
const MAX_STDOUT_BYTES: usize = 64 * 1024;

#[derive(Clone)]
struct AppState {
    engine: Engine,
}

struct HostState {
    wasi: WasiP1Ctx,
}

#[tokio::main]
async fn main() -> Result<()> {
    run().await
}

async fn run() -> Result<()> {
    let app = build_app(AppState {
        engine: Engine::default(),
    });

    let listener = tokio::net::TcpListener::bind(HOST_ADDRESS)
        .await
        .with_context(|| format!("failed to bind HTTP listener on {HOST_ADDRESS}"))?;

    axum::serve(listener, app)
        .await
        .context("axum server exited unexpectedly")
}

fn build_app(state: AppState) -> Router {
    Router::new().route("/*path", post(faas_handler)).with_state(state)
}

async fn faas_handler(
    State(state): State<AppState>,
    Path(path): Path<String>,
    body: Bytes,
) -> impl IntoResponse {
    let Some(function_name) = resolve_function_name(&path) else {
        return (
            StatusCode::NOT_FOUND,
            format!("no guest function could be resolved from `{path}`"),
        )
            .into_response();
    };

    let engine = state.engine.clone();
    let result = tokio::task::spawn_blocking(move || execute_guest(&engine, &function_name, body))
        .await;

    match result {
        Ok(Ok(stdout)) => (StatusCode::OK, stdout).into_response(),
        Ok(Err(error)) => error_response(error).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("guest execution task failed: {error}"),
        )
            .into_response(),
    }
}

fn execute_guest(engine: &Engine, function_name: &str, body: Bytes) -> Result<Bytes> {
    let module_path = resolve_guest_module_path(function_name)?;
    let module = Module::from_file(engine, &module_path).map_err(|error| {
        anyhow!(
            "failed to load guest module from {}: {error}",
            module_path.display()
        )
    })?;
    let linker = build_linker(engine)?;
    let stdout = MemoryOutputPipe::new(MAX_STDOUT_BYTES);
    let wasi = WasiCtxBuilder::new()
        .stdin(MemoryInputPipe::new(body))
        .stdout(stdout.clone())
        .build_p1();
    let mut store = Store::new(engine, HostState { wasi });
    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|error| anyhow!("failed to instantiate guest module: {error}"))?;
    let faas_entry: TypedFunc<(), ()> = instance
        .get_typed_func(&mut store, "faas_entry")
        .map_err(|error| anyhow!("failed to resolve exported function `faas_entry`: {error}"))?;

    faas_entry
        .call(&mut store, ())
        .map_err(|error| anyhow!("guest function `faas_entry` trapped: {error}"))?;

    Ok(stdout.contents())
}

fn error_response(error: anyhow::Error) -> (StatusCode, String) {
    let message = error.to_string();
    let status = if message.starts_with("guest module not found") {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    (status, message)
}

fn build_linker(engine: &Engine) -> Result<Linker<HostState>> {
    let mut linker = Linker::new(engine);
    p1::add_to_linker_sync(&mut linker, |state: &mut HostState| &mut state.wasi)
        .map_err(|error| anyhow!("failed to add WASI preview1 functions to linker: {error}"))?;
    Ok(linker)
}

fn resolve_function_name(path: &str) -> Option<String> {
    path.split('/')
        .rev()
        .find(|segment| !segment.is_empty() && *segment != "api")
        .map(ToOwned::to_owned)
}

fn resolve_guest_module_path(function_name: &str) -> Result<PathBuf> {
    let wasm_file = format!("{}.wasm", function_name.replace('-', "_"));
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidate_strings = [
        format!("../target/wasm32-wasip1/debug/{wasm_file}"),
        format!("../target/wasm32-wasi/debug/{wasm_file}"),
        format!("target/wasm32-wasip1/debug/{wasm_file}"),
        format!("target/wasm32-wasi/debug/{wasm_file}"),
    ];
    let candidates = [
        manifest_dir.join(&candidate_strings[0]),
        manifest_dir.join(&candidate_strings[1]),
        PathBuf::from(&candidate_strings[2]),
        PathBuf::from(&candidate_strings[3]),
    ];

    candidates
        .into_iter()
        .find(|path| path.exists())
        .map(normalize_path)
        .ok_or_else(|| {
            anyhow!(
                "guest module not found for `{function_name}`; expected one of: {}",
                format_candidate_list(&candidate_strings)
            )
        })
}

fn normalize_path(path: PathBuf) -> PathBuf {
    path.canonicalize().unwrap_or(path)
}

fn format_candidate_list(paths: &[String]) -> String {
    paths.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::util::ServiceExt;

    #[test]
    fn execute_guest_returns_stdout_payload() {
        let engine = Engine::default();
        let response =
            execute_guest(&engine, "guest-example", Bytes::from("Hello Lean FaaS!"))
                .expect("guest execution should succeed");

        assert_eq!(
            String::from_utf8_lossy(&response).trim(),
            "FaaS received: Hello Lean FaaS!"
        );
    }

    #[tokio::test]
    async fn router_returns_guest_stdout_for_post_request() {
        let app = build_app(AppState {
            engine: Engine::default(),
        });
        let response = app
            .oneshot(
                Request::post("/api/guest-example")
                    .body(Body::from("Hello Lean FaaS!"))
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::OK);

        let body = response
            .into_body()
            .collect()
            .await
            .expect("response body should collect")
            .to_bytes();

        assert_eq!(
            String::from_utf8_lossy(&body).trim(),
            "FaaS received: Hello Lean FaaS!"
        );
    }
}

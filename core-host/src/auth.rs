use anyhow::{anyhow, Context, Result};
use axum::{
    extract::{Request, State},
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use wasmtime::{
    component::{Component, Linker as ComponentLinker},
    Engine, Store,
};
use wasmtime_wasi::{
    DirPerms, FilePerms, ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView,
};

mod bindings {
    wasmtime::component::bindgen!({
        path: "../wit/identity.wit",
        world: "auth-guest",
    });
}

const DEFAULT_JWT_SECRET: &str = "tachyon-dev-secret";
const JWT_SECRET_ENV: &str = "TACHYON_AUTH_JWT_SECRET";

use bindings::exports::tachyon::identity::auth::AuthError;

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(crate) struct AuthClaims {
    pub(crate) subject: String,
    pub(crate) roles: Vec<String>,
}

#[derive(Clone)]
pub(crate) struct AuthManager {
    module_name: String,
    state_dir: PathBuf,
    jwt_secret: String,
}

struct AuthComponentState {
    ctx: WasiCtx,
    table: ResourceTable,
}

#[derive(Debug)]
pub(crate) enum AuthFailure {
    Unauthorized(String),
    Forbidden(String),
    Internal(String),
}

impl AuthFailure {
    pub(crate) fn into_response(self) -> Response {
        match self {
            Self::Unauthorized(message) => (StatusCode::UNAUTHORIZED, message).into_response(),
            Self::Forbidden(message) => (StatusCode::FORBIDDEN, message).into_response(),
            Self::Internal(message) => (StatusCode::INTERNAL_SERVER_ERROR, message).into_response(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct RecoveryCodeRequest {
    username: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct RecoveryCodeResponse {
    codes: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConsumeRecoveryCodeRequest {
    username: String,
    code: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ConsumeRecoveryCodeResponse {
    token: String,
}

pub(crate) fn auth_state_dir(manifest_path: &Path) -> PathBuf {
    manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("auth-state")
}

impl AuthManager {
    pub(crate) fn new(manifest_path: &Path) -> Result<Self> {
        let state_dir = auth_state_dir(manifest_path);
        fs::create_dir_all(&state_dir).with_context(|| {
            format!(
                "failed to initialize auth state directory `{}`",
                state_dir.display()
            )
        })?;

        Ok(Self {
            module_name: "system-faas-auth".to_owned(),
            state_dir,
            jwt_secret: std::env::var(JWT_SECRET_ENV)
                .unwrap_or_else(|_| DEFAULT_JWT_SECRET.to_owned()),
        })
    }

    pub(crate) fn verify_token(
        &self,
        engine: &Engine,
        token: &str,
        required_roles: &[&str],
    ) -> Result<AuthClaims, AuthFailure> {
        let (mut store, bindings) = self
            .instantiate(engine)
            .map_err(|error| AuthFailure::Internal(error.to_string()))?;
        let required_roles = required_roles
            .iter()
            .map(|role| (*role).to_owned())
            .collect::<Vec<_>>();
        let result = bindings
            .tachyon_identity_auth()
            .call_verify_token(&mut store, token, &required_roles)
            .map_err(|error| AuthFailure::Internal(format!("auth component trapped: {error}")))?;

        result
            .map(|claims| AuthClaims {
                subject: claims.subject,
                roles: claims.roles,
            })
            .map_err(map_auth_error)
    }

    #[allow(dead_code)]
    pub(crate) fn generate_recovery_codes(
        &self,
        engine: &Engine,
        username: &str,
    ) -> Result<Vec<String>> {
        let (mut store, bindings) = self.instantiate(engine)?;
        bindings
            .tachyon_identity_auth()
            .call_generate_recovery_codes(&mut store, username)
            .map_err(|error| {
                anyhow!("auth component trapped while generating recovery codes: {error}")
            })?
            .map_err(|error| anyhow!(error))
    }

    #[allow(dead_code)]
    pub(crate) fn consume_recovery_code(
        &self,
        engine: &Engine,
        username: &str,
        code: &str,
    ) -> Result<String> {
        let (mut store, bindings) = self.instantiate(engine)?;
        bindings
            .tachyon_identity_auth()
            .call_consume_recovery_code(&mut store, username, code)
            .map_err(|error| {
                anyhow!("auth component trapped while consuming recovery code: {error}")
            })?
            .map_err(|error| anyhow!(error))
    }

    fn instantiate(
        &self,
        engine: &Engine,
    ) -> Result<(Store<AuthComponentState>, bindings::AuthGuest)> {
        fs::create_dir_all(&self.state_dir).with_context(|| {
            format!(
                "failed to initialize auth state directory `{}`",
                self.state_dir.display()
            )
        })?;
        let module_path = crate::resolve_guest_module_path(&self.module_name)
            .map_err(|error| anyhow!(error.to_string()))?;
        let component = Component::from_file(engine, &module_path).map_err(|error| {
            anyhow!(
                "failed to load auth component from `{}`: {error}",
                module_path.display()
            )
        })?;
        let mut linker = ComponentLinker::new(engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|error| {
            anyhow!("failed to add WASI preview2 functions to auth component linker: {error}")
        })?;

        let mut wasi = WasiCtxBuilder::new();
        wasi.env(JWT_SECRET_ENV, &self.jwt_secret);
        wasi.preopened_dir(
            &self.state_dir,
            ".",
            DirPerms::READ | DirPerms::MUTATE,
            FilePerms::READ | FilePerms::WRITE,
        )
        .map_err(|error| {
            anyhow!(
                "failed to preopen auth state directory `{}`: {error}",
                self.state_dir.display()
            )
        })?;

        let mut store = Store::new(
            engine,
            AuthComponentState {
                ctx: wasi.build(),
                table: ResourceTable::new(),
            },
        );
        let bindings = bindings::AuthGuest::instantiate(&mut store, &component, &linker)
            .map_err(|error| anyhow!("failed to instantiate auth component: {error}"))?;
        Ok((store, bindings))
    }
}

pub(crate) async fn admin_auth_middleware(
    State(state): State<crate::AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let token = match bearer_token(request.headers()) {
        Ok(token) => token,
        Err(error) => return error.into_response(),
    };
    let auth_manager = Arc::clone(&state.auth_manager);
    let engine = state.runtime.load().engine.clone();

    let claims = match tokio::task::spawn_blocking(move || {
        auth_manager.verify_token(&engine, &token, &["admin"])
    })
    .await
    {
        Ok(Ok(claims)) => claims,
        Ok(Err(error)) => return error.into_response(),
        Err(error) => {
            return AuthFailure::Internal(format!(
                "failed to join auth verification task: {error}"
            ))
            .into_response();
        }
    };

    request.extensions_mut().insert(claims);
    next.run(request).await
}

pub(crate) async fn admin_status_handler(State(state): State<crate::AppState>) -> String {
    let runtime = state.runtime.load();
    format!(
        "routes={} batch_targets={} status=ready",
        runtime.config.routes.len(),
        runtime.config.batch_targets.len()
    )
}

pub(crate) async fn generate_recovery_codes_handler(
    State(state): State<crate::AppState>,
    Json(payload): Json<RecoveryCodeRequest>,
) -> Result<Json<RecoveryCodeResponse>, Response> {
    let auth_manager = Arc::clone(&state.auth_manager);
    let engine = state.runtime.load().engine.clone();
    let username = payload.username;

    let codes = tokio::task::spawn_blocking(move || {
        auth_manager.generate_recovery_codes(&engine, &username)
    })
    .await
    .map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to join recovery code generation task: {error}"),
        )
            .into_response()
    })?
    .map_err(string_error_to_response)?;

    Ok(Json(RecoveryCodeResponse { codes }))
}

pub(crate) async fn consume_recovery_code_handler(
    State(state): State<crate::AppState>,
    Json(payload): Json<ConsumeRecoveryCodeRequest>,
) -> Result<Json<ConsumeRecoveryCodeResponse>, Response> {
    let auth_manager = Arc::clone(&state.auth_manager);
    let engine = state.runtime.load().engine.clone();
    let username = payload.username;
    let code = payload.code;

    let token = tokio::task::spawn_blocking(move || {
        auth_manager.consume_recovery_code(&engine, &username, &code)
    })
    .await
    .map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to join recovery code consumption task: {error}"),
        )
            .into_response()
    })?
    .map_err(string_error_to_response)?;

    Ok(Json(ConsumeRecoveryCodeResponse { token }))
}

impl WasiView for AuthComponentState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.ctx,
            table: &mut self.table,
        }
    }
}

impl wasmtime::component::HasData for AuthComponentState {
    type Data<'a> = &'a mut Self;
}

pub(crate) fn bearer_token(headers: &HeaderMap) -> Result<String, AuthFailure> {
    let value = headers
        .get(AUTHORIZATION)
        .ok_or_else(|| AuthFailure::Unauthorized("missing Authorization header".to_owned()))?;
    let value = value.to_str().map_err(|_| {
        AuthFailure::Unauthorized("Authorization header is not valid UTF-8".to_owned())
    })?;
    value
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            AuthFailure::Unauthorized("Authorization header must use the Bearer scheme".to_owned())
        })
}

fn map_auth_error(error: AuthError) -> AuthFailure {
    match error {
        AuthError::Expired => AuthFailure::Unauthorized("token has expired".to_owned()),
        AuthError::InvalidSignature => {
            AuthFailure::Unauthorized("token signature is invalid".to_owned())
        }
        AuthError::MissingRoles => {
            AuthFailure::Forbidden("token does not include the required role".to_owned())
        }
        AuthError::InternalError(message) => AuthFailure::Internal(message),
    }
}

fn string_error_to_response(error: anyhow::Error) -> Response {
    let message = error.to_string();
    let status = if message.contains("must not be empty")
        || message.contains("must match")
        || message.contains("invalid")
    {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };

    (status, message).into_response()
}

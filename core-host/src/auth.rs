use anyhow::{anyhow, Context, Result};
use axum::{
    extract::{Extension, Request, State},
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

mod authn_bindings {
    wasmtime::component::bindgen!({
        path: "../wit/authn.wit",
        world: "authn-guest",
    });
}

mod authz_bindings {
    wasmtime::component::bindgen!({
        path: "../wit/authz.wit",
        world: "authz-guest",
    });
}

const DEFAULT_JWT_SECRET: &str = "tachyon-dev-secret";
const JWT_SECRET_ENV: &str = "TACHYON_AUTH_JWT_SECRET";
const AUTH_STATE_DIR_ENV: &str = "TACHYON_AUTH_STATE_DIR";
const DEFAULT_PAT_TTL_DAYS: u32 = 30;

use authn_bindings::exports::tachyon::identity::authn::AuthnError;
use authz_bindings::exports::tachyon::identity::authz::AuthzError;

#[derive(Clone, Debug)]
pub(crate) struct AuthClaims {
    pub(crate) subject: String,
    pub(crate) roles: Vec<String>,
    pub(crate) scopes: Vec<String>,
}

#[derive(Clone)]
pub(crate) struct AuthManager {
    authn_module_name: String,
    authz_module_name: String,
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

#[derive(Debug, Deserialize)]
pub(crate) struct IssuePatRequest {
    name: String,
    #[serde(default)]
    scopes: Vec<String>,
    #[serde(default = "default_pat_ttl_days")]
    ttl_days: u32,
}

#[derive(Debug, Serialize)]
pub(crate) struct IssuePatResponse {
    token: String,
}

fn default_pat_ttl_days() -> u32 {
    DEFAULT_PAT_TTL_DAYS
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
            authn_module_name: "system-faas-authn".to_owned(),
            authz_module_name: "system-faas-authz".to_owned(),
            state_dir,
            jwt_secret: std::env::var(JWT_SECRET_ENV)
                .unwrap_or_else(|_| DEFAULT_JWT_SECRET.to_owned()),
        })
    }

    pub(crate) fn authorize_request(
        &self,
        engine: &Engine,
        token: &str,
        method: &str,
        path: &str,
    ) -> Result<AuthClaims, AuthFailure> {
        let claims = self.authenticate(engine, token)?;
        self.authorize(engine, &claims, method, path)?;
        Ok(claims)
    }

    pub(crate) fn generate_recovery_codes(
        &self,
        engine: &Engine,
        username: &str,
    ) -> Result<Vec<String>> {
        let (mut store, bindings) = self.instantiate_authn(engine)?;
        bindings
            .tachyon_identity_authn()
            .call_generate_recovery_codes(&mut store, username)
            .map_err(|error| {
                anyhow!("authn component trapped while generating recovery codes: {error}")
            })?
            .map_err(|error| anyhow!(error))
    }

    pub(crate) fn consume_recovery_code(
        &self,
        engine: &Engine,
        username: &str,
        code: &str,
    ) -> Result<String> {
        let (mut store, bindings) = self.instantiate_authn(engine)?;
        bindings
            .tachyon_identity_authn()
            .call_consume_recovery_code(&mut store, username, code)
            .map_err(|error| {
                anyhow!("authn component trapped while consuming recovery code: {error}")
            })?
            .map_err(|error| anyhow!(error))
    }

    pub(crate) fn issue_pat(
        &self,
        engine: &Engine,
        subject: &str,
        name: &str,
        scopes: &[String],
        ttl_days: u32,
    ) -> Result<String> {
        let (mut store, bindings) = self.instantiate_authn(engine)?;
        bindings
            .tachyon_identity_authn()
            .call_issue_pat(&mut store, subject, name, scopes, ttl_days)
            .map_err(|error| anyhow!("authn component trapped while issuing PAT: {error}"))?
            .map_err(|error| anyhow!(error))
    }

    fn authenticate(&self, engine: &Engine, token: &str) -> Result<AuthClaims, AuthFailure> {
        let (mut store, bindings) = self
            .instantiate_authn(engine)
            .map_err(|error| AuthFailure::Internal(error.to_string()))?;
        let result = bindings
            .tachyon_identity_authn()
            .call_validate_token(&mut store, token)
            .map_err(|error| AuthFailure::Internal(format!("authn component trapped: {error}")))?;

        result
            .map(|claims| AuthClaims {
                subject: claims.subject,
                roles: claims.roles,
                scopes: claims.scopes,
            })
            .map_err(map_authn_error)
    }

    fn authorize(
        &self,
        engine: &Engine,
        claims: &AuthClaims,
        method: &str,
        path: &str,
    ) -> Result<(), AuthFailure> {
        let (mut store, bindings) = self
            .instantiate_authz(engine)
            .map_err(|error| AuthFailure::Internal(error.to_string()))?;
        let identity = authz_bindings::exports::tachyon::identity::authz::IdentityPayload {
            subject: claims.subject.clone(),
            roles: claims.roles.clone(),
            scopes: claims.scopes.clone(),
        };
        let result = bindings
            .tachyon_identity_authz()
            .call_evaluate_policy(&mut store, &identity, method, path)
            .map_err(|error| AuthFailure::Internal(format!("authz component trapped: {error}")))?;

        let allowed = result.map_err(map_authz_error)?;
        if allowed {
            Ok(())
        } else {
            Err(AuthFailure::Forbidden(format!(
                "the authenticated identity is not allowed to access `{path}`"
            )))
        }
    }

    fn instantiate_authn(
        &self,
        engine: &Engine,
    ) -> Result<(Store<AuthComponentState>, authn_bindings::AuthnGuest)> {
        fs::create_dir_all(&self.state_dir).with_context(|| {
            format!(
                "failed to initialize auth state directory `{}`",
                self.state_dir.display()
            )
        })?;
        let module_path = crate::resolve_guest_module_path(&self.authn_module_name)
            .map_err(|error| anyhow!(error.to_string()))?;
        let component = Component::from_file(engine, &module_path).map_err(|error| {
            anyhow!(
                "failed to load authn component from `{}`: {error}",
                module_path.display()
            )
        })?;
        let mut linker = ComponentLinker::new(engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|error| {
            anyhow!("failed to add WASI preview2 functions to authn component linker: {error}")
        })?;

        let mut wasi = WasiCtxBuilder::new();
        wasi.env(JWT_SECRET_ENV, &self.jwt_secret);
        wasi.env(AUTH_STATE_DIR_ENV, ".");
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
        let bindings = authn_bindings::AuthnGuest::instantiate(&mut store, &component, &linker)
            .map_err(|error| anyhow!("failed to instantiate authn component: {error}"))?;
        Ok((store, bindings))
    }

    fn instantiate_authz(
        &self,
        engine: &Engine,
    ) -> Result<(Store<AuthComponentState>, authz_bindings::AuthzGuest)> {
        let module_path = crate::resolve_guest_module_path(&self.authz_module_name)
            .map_err(|error| anyhow!(error.to_string()))?;
        let component = Component::from_file(engine, &module_path).map_err(|error| {
            anyhow!(
                "failed to load authz component from `{}`: {error}",
                module_path.display()
            )
        })?;
        let mut linker = ComponentLinker::new(engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|error| {
            anyhow!("failed to add WASI preview2 functions to authz component linker: {error}")
        })?;

        let mut store = Store::new(
            engine,
            AuthComponentState {
                ctx: WasiCtxBuilder::new().build(),
                table: ResourceTable::new(),
            },
        );
        let bindings = authz_bindings::AuthzGuest::instantiate(&mut store, &component, &linker)
            .map_err(|error| anyhow!("failed to instantiate authz component: {error}"))?;
        Ok((store, bindings))
    }
}

#[allow(dead_code)]
pub(crate) async fn authorize_admin_headers(
    state: &crate::AppState,
    method: &str,
    path: &str,
    headers: &HeaderMap,
) -> Option<Response> {
    if !path.starts_with("/admin/") {
        return None;
    }

    let token = match bearer_token(headers) {
        Ok(token) => token,
        Err(error) => return Some(error.into_response()),
    };
    let auth_manager = Arc::clone(&state.auth_manager);
    let engine = state.runtime.load().engine.clone();
    let method = method.to_owned();
    let path = path.to_owned();

    match tokio::task::spawn_blocking(move || {
        auth_manager.authorize_request(&engine, &token, &method, &path)
    })
    .await
    {
        Ok(Ok(_)) => None,
        Ok(Err(error)) => Some(error.into_response()),
        Err(error) => Some(
            AuthFailure::Internal(format!("failed to join auth pipeline task: {error}"))
                .into_response(),
        ),
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
    let method = request.method().as_str().to_owned();
    let path = request.uri().path().to_owned();

    let claims = match tokio::task::spawn_blocking(move || {
        auth_manager.authorize_request(&engine, &token, &method, &path)
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

pub(crate) async fn regenerate_account_security_handler(
    State(state): State<crate::AppState>,
    Extension(claims): Extension<AuthClaims>,
) -> Result<Json<RecoveryCodeResponse>, Response> {
    let auth_manager = Arc::clone(&state.auth_manager);
    let engine = state.runtime.load().engine.clone();
    let username = claims.subject;

    let codes = tokio::task::spawn_blocking(move || {
        auth_manager.generate_recovery_codes(&engine, &username)
    })
    .await
    .map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to join account security regeneration task: {error}"),
        )
            .into_response()
    })?
    .map_err(string_error_to_response)?;

    Ok(Json(RecoveryCodeResponse { codes }))
}

pub(crate) async fn issue_pat_handler(
    State(state): State<crate::AppState>,
    Extension(claims): Extension<AuthClaims>,
    Json(payload): Json<IssuePatRequest>,
) -> Result<Json<IssuePatResponse>, Response> {
    let auth_manager = Arc::clone(&state.auth_manager);
    let engine = state.runtime.load().engine.clone();
    let subject = claims.subject;
    let IssuePatRequest {
        name,
        scopes,
        ttl_days,
    } = payload;

    let token = tokio::task::spawn_blocking(move || {
        auth_manager.issue_pat(&engine, &subject, &name, &scopes, ttl_days)
    })
    .await
    .map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to join PAT issuance task: {error}"),
        )
            .into_response()
    })?
    .map_err(string_error_to_response)?;

    Ok(Json(IssuePatResponse { token }))
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

fn map_authn_error(error: AuthnError) -> AuthFailure {
    match error {
        AuthnError::Expired => AuthFailure::Unauthorized("token has expired".to_owned()),
        AuthnError::InvalidCredential => AuthFailure::Unauthorized(
            "token is malformed, unknown, or has an invalid signature".to_owned(),
        ),
        AuthnError::InternalError(message) => AuthFailure::Internal(message),
    }
}

fn map_authz_error(error: AuthzError) -> AuthFailure {
    match error {
        AuthzError::InternalError(message) => AuthFailure::Internal(message),
    }
}

fn string_error_to_response(error: anyhow::Error) -> Response {
    let message = error.to_string();
    let status = if message.contains("must not be empty")
        || message.contains("must match")
        || message.contains("invalid")
        || message.contains("between 1 and")
    {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };

    (status, message).into_response()
}

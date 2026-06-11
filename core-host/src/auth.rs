use anyhow::{anyhow, Context, Result};
use axum::{
    extract::{Extension, Request, State},
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
#[cfg(feature = "s3-persistence")]
use std::time::Duration;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;
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

const JWT_SECRET_ENV: &str = "TACHYON_AUTH_JWT_SECRET";
const JWT_SECRET_FILE: &str = "jwt.secret";
const AUTH_STATE_DIR_ENV: &str = "TACHYON_AUTH_STATE_DIR";
#[cfg(feature = "s3-persistence")]
const AUTH_STATE_FLUSH_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_PAT_TTL_DAYS: u32 = 30;

use authn_bindings::exports::tachyon::identity::authn::{
    AuthSession as AuthnSession, AuthnError, GroupInput as AuthnGroupInput,
    GroupSummary as AuthnGroupSummary, RegistrationTokenClaims as AuthnRegistrationTokenClaims,
    SignupProfile as AuthnSignupProfile, StagedLoginSession as AuthnStagedLoginSession,
    StagedUserSession as AuthnStagedUserSession, UserSummary as AuthnUserSummary,
    UserUpdate as AuthnUserUpdate,
};
use authz_bindings::exports::tachyon::identity::authz::AuthzError;

#[derive(Clone, Debug)]
pub(crate) struct AuthClaims {
    pub(crate) subject: String,
    pub(crate) roles: Vec<String>,
    pub(crate) scopes: Vec<String>,
}

/// In-process cache of full authn+authz decisions, keyed by SHA-256(token) plus the
/// (method, path) the caller wanted to access. Hashing the token keeps the raw
/// secret out of the cache key space — a memory dump exposes only the digest.
///
/// Bounded to 16 384 entries so a token-spoofing flood cannot OOM the host. Time-
/// to-idle of 5 minutes is well below the typical PAT lifetime; mutations issued
/// via `system-faas-authz` invalidate matching entries through the
/// `authz_purge_outbox` table, so the steady-state worst case is "5 minutes of
/// stale access" only when the host is also network-partitioned from its own
/// outbox storage, which is impossible by construction (redb is in-process).
#[derive(Clone)]
pub(crate) struct AuthDecisionCache {
    inner: moka::sync::Cache<AuthDecisionKey, AuthDecision>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct AuthDecisionKey {
    token_hash: [u8; 32],
    method: String,
    path: String,
}

#[derive(Clone, Debug)]
struct AuthDecision {
    claims: AuthClaims,
}

impl AuthDecisionCache {
    pub(crate) fn new() -> Self {
        use std::time::Duration;
        Self {
            inner: moka::sync::Cache::builder()
                .max_capacity(16_384)
                .time_to_idle(Duration::from_secs(300))
                .support_invalidation_closures()
                .build(),
        }
    }

    fn key(token: &str, method: &str, path: &str) -> AuthDecisionKey {
        use sha2::Digest;
        let digest = sha2::Sha256::digest(token.as_bytes());
        let mut token_hash = [0u8; 32];
        token_hash.copy_from_slice(digest.as_slice());
        AuthDecisionKey {
            token_hash,
            method: method.to_owned(),
            path: path.to_owned(),
        }
    }

    fn get(&self, token: &str, method: &str, path: &str) -> Option<AuthClaims> {
        self.inner
            .get(&Self::key(token, method, path))
            .map(|d| d.claims)
    }

    fn put(&self, token: &str, method: &str, path: &str, claims: AuthClaims) {
        self.inner
            .insert(Self::key(token, method, path), AuthDecision { claims });
    }

    /// Invalidate every cached entry that derived from the given token. Called from
    /// the authz purge subscriber after a token revoke / role change / user ban.
    pub(crate) fn invalidate_token(&self, token_hash: &[u8; 32]) {
        let target = *token_hash;
        self.inner.invalidate_entries_if(move |key, _| {
            key.token_hash == target
        }).expect("invalidate_entries_if registers a predicate; failure here would mean moka was misconfigured");
    }

    /// Invalidate every cached entry whose claims include the given subject. Used
    /// for role-update / ban events that arrive without a specific token hash.
    pub(crate) fn invalidate_subject(&self, subject: &str) {
        let owned = subject.to_owned();
        self.inner.invalidate_entries_if(move |_, decision| {
            decision.claims.subject == owned
        }).expect("invalidate_entries_if registers a predicate; failure here would mean moka was misconfigured");
    }

    #[cfg(test)]
    #[cfg(feature = "experimental")]
    pub(crate) fn entry_count(&self) -> u64 {
        self.inner.run_pending_tasks();
        self.inner.entry_count()
    }
}

impl Default for AuthDecisionCache {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub(crate) struct AuthManager {
    authn_module_name: String,
    authz_module_name: String,
    state_dir: PathBuf,
    jwt_secret: String,
    decision_cache: AuthDecisionCache,
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ValidateRegistrationTokenRequest {
    token: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RegistrationTokenClaimsResponse {
    subject: String,
    roles: Vec<String>,
    scopes: Vec<String>,
    expires_at: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StageSignupRequest {
    token: String,
    first_name: String,
    last_name: String,
    username: String,
    password: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StagedUserSessionResponse {
    session_id: String,
    username: String,
    provisioning_uri: String,
    roles: Vec<String>,
    scopes: Vec<String>,
    expires_at: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StagedLoginSessionResponse {
    session_id: String,
    username: String,
    expires_at: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FinalizeEnrollmentRequest {
    session_id: String,
    totp_code: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StageLoginRequest {
    username: String,
    password: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FinalizeLoginRequest {
    session_id: String,
    totp_code: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StepUpSessionRequest {
    totp_code: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StepUpSessionResponse {
    mfa_session_token: String,
    expires_at: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FinalizeEnrollmentResponse {
    token: String,
    username: String,
    roles: Vec<String>,
    scopes: Vec<String>,
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IamUserSummaryResponse {
    pub(crate) username: String,
    pub(crate) first_name: String,
    pub(crate) last_name: String,
    pub(crate) roles: Vec<String>,
    pub(crate) scopes: Vec<String>,
    pub(crate) groups: Vec<String>,
    pub(crate) disabled_at: Option<u64>,
    pub(crate) created_at: u64,
    pub(crate) last_login_at: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IamGroupSummaryResponse {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) roles: Vec<String>,
    pub(crate) scopes: Vec<String>,
    pub(crate) member_count: u32,
    pub(crate) created_at: u64,
    pub(crate) updated_at: u64,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateUserRequest {
    #[serde(default)]
    pub(crate) add_groups: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) remove_groups: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) add_roles: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) remove_roles: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) add_scopes: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) remove_scopes: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) disabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpsertGroupRequest {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) description: String,
    #[serde(default)]
    pub(crate) roles: Vec<String>,
    #[serde(default)]
    pub(crate) scopes: Vec<String>,
}

fn default_pat_ttl_days() -> u32 {
    DEFAULT_PAT_TTL_DAYS
}

pub(crate) fn auth_state_dir(manifest_path: &Path) -> PathBuf {
    if let Some(path) = std::env::var_os(AUTH_STATE_DIR_ENV).filter(|value| !value.is_empty()) {
        return PathBuf::from(path);
    }

    manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("auth-state")
}

/// Resolve the HS256 signing secret used for identity tokens.
///
/// Fail-secure — never a constant compiled into the binary (the same rationale
/// as the TDE volume key in `component_hosts::tde_key_bytes`). Resolution order:
///
/// 1. `TACHYON_AUTH_JWT_SECRET` when set and non-empty. This is **required** for
///    multi-node meshes, where every node must share the same secret for tokens
///    to validate across nodes (see `manifests/deploy-mesh.yaml`).
/// 2. A secret previously persisted under `<state_dir>/jwt.secret`, so a single
///    node keeps a stable secret across restarts.
/// 3. A freshly generated 256-bit random secret, persisted for restart
///    stability. A warning is emitted because a per-node generated secret cannot
///    be shared across a mesh.
fn resolve_jwt_secret(state_dir: &Path) -> String {
    if let Ok(secret) = std::env::var(JWT_SECRET_ENV) {
        if !secret.trim().is_empty() {
            return secret;
        }
    }

    let secret_path = state_dir.join(JWT_SECRET_FILE);
    if let Ok(persisted) = fs::read_to_string(&secret_path) {
        let persisted = persisted.trim();
        if !persisted.is_empty() {
            return persisted.to_owned();
        }
    }

    let generated = hex::encode(rand::random::<[u8; 32]>());
    match persist_jwt_secret(&secret_path, &generated) {
        Ok(()) => tracing::warn!(
            secret_path = %secret_path.display(),
            "{JWT_SECRET_ENV} is not set; generated a random per-node JWT secret. \
             Set {JWT_SECRET_ENV} to a shared value for multi-node deployments."
        ),
        Err(error) => tracing::warn!(
            %error,
            "{JWT_SECRET_ENV} is not set and the generated secret could not be \
             persisted; tokens will not survive a restart. Set {JWT_SECRET_ENV} \
             explicitly."
        ),
    }
    generated
}

/// Persist the generated JWT secret with owner-only permissions where the
/// platform supports it. Best-effort: a failure here is surfaced by the caller
/// as a warning, not a hard error.
fn persist_jwt_secret(path: &Path, secret: &str) -> std::io::Result<()> {
    fs::write(path, secret)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
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
            jwt_secret: resolve_jwt_secret(&state_dir),
            state_dir,
            decision_cache: AuthDecisionCache::new(),
        })
    }

    /// Expose the in-process decision cache so the host's `authz_purge_outbox`
    /// subscriber can invalidate entries on token revocations / role updates / bans.
    pub(crate) fn decision_cache(&self) -> &AuthDecisionCache {
        &self.decision_cache
    }

    pub(crate) fn authorize_request(
        &self,
        engine: &Engine,
        token: &str,
        method: &str,
        path: &str,
    ) -> Result<AuthClaims, AuthFailure> {
        if let Some(cached) = self.decision_cache.get(token, method, path) {
            return Ok(cached);
        }
        let claims = self.authenticate(engine, token)?;
        self.authorize(engine, &claims, method, path)?;
        // Only positive decisions are cached. A `Forbidden` outcome is left out so a
        // subsequent role change that *grants* access takes effect immediately
        // without waiting for an authz_purge_outbox round-trip.
        self.decision_cache.put(token, method, path, claims.clone());
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

    pub(crate) fn validate_registration_token(
        &self,
        engine: &Engine,
        token: &str,
    ) -> Result<RegistrationTokenClaimsResponse> {
        let (mut store, bindings) = self.instantiate_authn(engine)?;
        let claims = bindings
            .tachyon_identity_authn()
            .call_validate_registration_token(&mut store, token)
            .map_err(|error| {
                anyhow!("authn component trapped while validating registration token: {error}")
            })?
            .map_err(|error| anyhow!(error))?;

        Ok(map_registration_claims(claims))
    }

    pub(crate) fn stage_user(
        &self,
        engine: &Engine,
        request: StageSignupRequest,
    ) -> Result<StagedUserSessionResponse> {
        let (mut store, bindings) = self.instantiate_authn(engine)?;
        let session = bindings
            .tachyon_identity_authn()
            .call_stage_user(
                &mut store,
                &request.token,
                &AuthnSignupProfile {
                    first_name: request.first_name,
                    last_name: request.last_name,
                    username: request.username,
                    password: request.password,
                },
            )
            .map_err(|error| anyhow!("authn component trapped while staging user: {error}"))?
            .map_err(|error| anyhow!(error))?;

        Ok(map_staged_user_session(session))
    }

    pub(crate) fn finalize_enrollment(
        &self,
        engine: &Engine,
        session_id: &str,
        totp_code: &str,
    ) -> Result<FinalizeEnrollmentResponse> {
        let (mut store, bindings) = self.instantiate_authn(engine)?;
        let session = bindings
            .tachyon_identity_authn()
            .call_finalize_enrollment(&mut store, session_id, totp_code)
            .map_err(|error| {
                anyhow!("authn component trapped while finalizing enrollment: {error}")
            })?
            .map_err(|error| anyhow!(error))?;

        Ok(map_auth_session(session))
    }

    pub(crate) fn stage_login(
        &self,
        engine: &Engine,
        username: &str,
        password: &str,
    ) -> Result<StagedLoginSessionResponse> {
        let (mut store, bindings) = self.instantiate_authn(engine)?;
        let session = bindings
            .tachyon_identity_authn()
            .call_stage_login(&mut store, username, password)
            .map_err(|error| anyhow!("authn component trapped while staging login: {error}"))?
            .map_err(|error| anyhow!(error))?;

        Ok(map_staged_login_session(session))
    }

    pub(crate) fn finalize_login(
        &self,
        engine: &Engine,
        session_id: &str,
        totp_code: &str,
    ) -> Result<FinalizeEnrollmentResponse> {
        let (mut store, bindings) = self.instantiate_authn(engine)?;
        let session = bindings
            .tachyon_identity_authn()
            .call_finalize_login(&mut store, session_id, totp_code)
            .map_err(|error| anyhow!("authn component trapped while finalizing login: {error}"))?
            .map_err(|error| anyhow!(error))?;

        Ok(map_auth_session(session))
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

    pub(crate) fn list_users(&self, engine: &Engine) -> Result<Vec<IamUserSummaryResponse>> {
        let (mut store, bindings) = self.instantiate_authn(engine)?;
        let summaries = bindings
            .tachyon_identity_authn()
            .call_list_users(&mut store)
            .map_err(|error| anyhow!("authn component trapped while listing users: {error}"))?
            .map_err(|error| anyhow!(error))?;
        Ok(summaries.into_iter().map(map_user_summary).collect())
    }

    pub(crate) fn update_user(
        &self,
        engine: &Engine,
        actor: &str,
        username: &str,
        update: AuthnUserUpdate,
    ) -> Result<IamUserSummaryResponse> {
        let (mut store, bindings) = self.instantiate_authn(engine)?;
        let summary = bindings
            .tachyon_identity_authn()
            .call_update_user(&mut store, actor, username, &update)
            .map_err(|error| anyhow!("authn component trapped while updating user: {error}"))?
            .map_err(|error| anyhow!(error))?;
        Ok(map_user_summary(summary))
    }

    pub(crate) fn delete_user(&self, engine: &Engine, actor: &str, username: &str) -> Result<()> {
        let (mut store, bindings) = self.instantiate_authn(engine)?;
        bindings
            .tachyon_identity_authn()
            .call_delete_user(&mut store, actor, username)
            .map_err(|error| anyhow!("authn component trapped while deleting user: {error}"))?
            .map_err(|error| anyhow!(error))
    }

    pub(crate) fn list_groups(&self, engine: &Engine) -> Result<Vec<IamGroupSummaryResponse>> {
        let (mut store, bindings) = self.instantiate_authn(engine)?;
        let summaries = bindings
            .tachyon_identity_authn()
            .call_list_groups(&mut store)
            .map_err(|error| anyhow!("authn component trapped while listing groups: {error}"))?
            .map_err(|error| anyhow!(error))?;
        Ok(summaries.into_iter().map(map_group_summary).collect())
    }

    pub(crate) fn upsert_group(
        &self,
        engine: &Engine,
        input: AuthnGroupInput,
    ) -> Result<IamGroupSummaryResponse> {
        let (mut store, bindings) = self.instantiate_authn(engine)?;
        let summary = bindings
            .tachyon_identity_authn()
            .call_upsert_group(&mut store, &input)
            .map_err(|error| anyhow!("authn component trapped while upserting group: {error}"))?
            .map_err(|error| anyhow!(error))?;
        Ok(map_group_summary(summary))
    }

    pub(crate) fn delete_group(&self, engine: &Engine, name: &str) -> Result<()> {
        let (mut store, bindings) = self.instantiate_authn(engine)?;
        bindings
            .tachyon_identity_authn()
            .call_delete_group(&mut store, name)
            .map_err(|error| anyhow!("authn component trapped while deleting group: {error}"))?
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

#[cfg(test)]
pub(crate) fn test_auth_manager_with_modules(
    authn_module_name: &str,
    authz_module_name: &str,
    state_dir: PathBuf,
) -> AuthManager {
    AuthManager {
        authn_module_name: authn_module_name.to_owned(),
        authz_module_name: authz_module_name.to_owned(),
        state_dir,
        jwt_secret: "test-secret".to_owned(),
        decision_cache: AuthDecisionCache::new(),
    }
}

/// Event payload written into the `authz_purge_outbox` redb table. Producers
/// (`system-faas-authz` mutation paths) emit one of these whenever a token is
/// revoked, a role assignment changes, or a user is banned. The host's
/// background subscriber drains the table and evicts the matching entries from
/// the in-process `AuthDecisionCache`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(crate) enum AuthzPurgeEvent {
    /// A specific Personal Access Token was revoked. `token_hash` is the hex of the
    /// SHA-256 of the raw token; the producer is responsible for hashing it before
    /// emitting so the raw token never ends up on disk.
    Token { token_hash: String, ts_ms: u64 },
    /// A user's role assignment changed; invalidate every cache entry whose claims
    /// list this subject.
    Role { user_id: String, ts_ms: u64 },
    /// A user was banned or globally suspended; same eviction shape as `Role` but
    /// surfaces the kind to the audit log distinctly.
    UserBan { user_id: String, ts_ms: u64 },
}

impl AuthzPurgeEvent {
    /// Helper used by `system-faas-authz` mutation paths (and by tests) to
    /// serialize and durably append a purge event. The redb append returns the
    /// monotonic key; on host crash the row survives and is replayed on next boot.
    #[cfg(feature = "experimental")]
    pub(crate) fn enqueue(&self, store: &crate::store::CoreStore) -> Result<String> {
        let payload = serde_json::to_vec(self).context("failed to serialize authz purge event")?;
        store
            .append_outbox(crate::store::CoreStoreBucket::AuthzPurgeOutbox, &payload)
            .context("failed to append authz purge event to outbox")
    }
}

/// Apply a purge event to the in-process cache. Pure function so it's easy to
/// unit-test independent of the redb-backed driver loop.
pub(crate) fn apply_authz_purge(cache: &AuthDecisionCache, event: &AuthzPurgeEvent) -> Result<()> {
    match event {
        AuthzPurgeEvent::Token { token_hash, .. } => {
            let bytes = hex::decode(token_hash)
                .context("authz purge event token_hash must be hex-encoded")?;
            if bytes.len() != 32 {
                anyhow::bail!(
                    "authz purge event token_hash must decode to 32 bytes; got {}",
                    bytes.len()
                );
            }
            let mut buf = [0u8; 32];
            buf.copy_from_slice(&bytes);
            cache.invalidate_token(&buf);
        }
        AuthzPurgeEvent::Role { user_id, .. } | AuthzPurgeEvent::UserBan { user_id, .. } => {
            cache.invalidate_subject(user_id);
        }
    }
    Ok(())
}

#[cfg(feature = "experimental")]
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

/// `GET /admin/identity/public-key` — returns the node's stable Ed25519 public
/// key in hex so operators can register it in peer `trusted_signers` lists.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NodePublicKeyResponse {
    public_key: String,
}

pub(crate) async fn node_public_key_handler(
    State(state): State<crate::AppState>,
) -> axum::Json<NodePublicKeyResponse> {
    axum::Json(NodePublicKeyResponse {
        public_key: state.host_identity.public_key_hex.clone(),
    })
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

    flush_auth_state(&state).await;
    Ok(Json(RecoveryCodeResponse { codes }))
}

pub(crate) async fn validate_registration_token_handler(
    State(state): State<crate::AppState>,
    Json(payload): Json<ValidateRegistrationTokenRequest>,
) -> Result<Json<RegistrationTokenClaimsResponse>, Response> {
    let auth_manager = Arc::clone(&state.auth_manager);
    let engine = state.runtime.load().engine.clone();
    let token = payload.token;

    let claims = tokio::task::spawn_blocking(move || {
        auth_manager.validate_registration_token(&engine, &token)
    })
    .await
    .map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to join registration token validation task: {error}"),
        )
            .into_response()
    })?
    .map_err(string_error_to_response)?;

    Ok(Json(claims))
}

pub(crate) async fn stage_signup_handler(
    State(state): State<crate::AppState>,
    Json(payload): Json<StageSignupRequest>,
) -> Result<Json<StagedUserSessionResponse>, Response> {
    let auth_manager = Arc::clone(&state.auth_manager);
    let engine = state.runtime.load().engine.clone();

    let session = tokio::task::spawn_blocking(move || auth_manager.stage_user(&engine, payload))
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to join signup staging task: {error}"),
            )
                .into_response()
        })?
        .map_err(string_error_to_response)?;

    flush_auth_state(&state).await;
    Ok(Json(session))
}

pub(crate) async fn finalize_enrollment_handler(
    State(state): State<crate::AppState>,
    Json(payload): Json<FinalizeEnrollmentRequest>,
) -> Result<Json<FinalizeEnrollmentResponse>, Response> {
    let auth_manager = Arc::clone(&state.auth_manager);
    let engine = state.runtime.load().engine.clone();
    let session_id = payload.session_id;
    let totp_code = payload.totp_code;

    let session = tokio::task::spawn_blocking(move || {
        auth_manager.finalize_enrollment(&engine, &session_id, &totp_code)
    })
    .await
    .map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to join enrollment finalization task: {error}"),
        )
            .into_response()
    })?
    .map_err(string_error_to_response)?;

    flush_auth_state(&state).await;
    Ok(Json(session))
}

pub(crate) async fn stage_login_handler(
    State(state): State<crate::AppState>,
    Json(payload): Json<StageLoginRequest>,
) -> Result<Json<StagedLoginSessionResponse>, Response> {
    let auth_manager = Arc::clone(&state.auth_manager);
    let engine = state.runtime.load().engine.clone();
    let username = payload.username;
    let password = payload.password;
    let username_for_audit = username.clone();

    let session = tokio::task::spawn_blocking(move || {
        auth_manager.stage_login(&engine, &username, &password)
    })
    .await
    .map_err(|error| {
        state.iam_audit_log.record(
            username_for_audit.clone(),
            "login.stage",
            Some(username_for_audit.clone()),
            None,
            "error",
            error.to_string(),
        );
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to join login staging task: {error}"),
        )
            .into_response()
    })?
    .map_err(|error| {
        state.iam_audit_log.record(
            username_for_audit.clone(),
            "login.stage",
            Some(username_for_audit.clone()),
            None,
            "error",
            error.to_string(),
        );
        string_error_to_response(error)
    })?;

    state.iam_audit_log.record(
        username_for_audit.clone(),
        "login.stage",
        Some(username_for_audit),
        None,
        "ok",
        String::new(),
    );
    Ok(Json(session))
}

pub(crate) async fn finalize_login_handler(
    State(state): State<crate::AppState>,
    Json(payload): Json<FinalizeLoginRequest>,
) -> Result<Json<FinalizeEnrollmentResponse>, Response> {
    let auth_manager = Arc::clone(&state.auth_manager);
    let engine = state.runtime.load().engine.clone();
    let session_id = payload.session_id;
    let totp_code = payload.totp_code;
    let session_id_for_audit = session_id.clone();

    let session = tokio::task::spawn_blocking(move || {
        auth_manager.finalize_login(&engine, &session_id, &totp_code)
    })
    .await
    .map_err(|error| {
        state.iam_audit_log.record(
            "<unknown>",
            "login.finalize",
            None,
            None,
            "error",
            format!("session_id={} {error}", session_id_for_audit),
        );
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to join login finalization task: {error}"),
        )
            .into_response()
    })?
    .map_err(|error| {
        state.iam_audit_log.record(
            "<unknown>",
            "login.finalize",
            None,
            None,
            "error",
            format!("session_id={} {error}", session_id_for_audit),
        );
        string_error_to_response(error)
    })?;

    state.iam_audit_log.record(
        session.username.clone(),
        "login.finalize",
        Some(session.username.clone()),
        None,
        "ok",
        String::new(),
    );
    flush_auth_state(&state).await;
    Ok(Json(session))
}

pub(crate) async fn issue_step_up_session_handler(
    Extension(claims): Extension<AuthClaims>,
    Json(payload): Json<StepUpSessionRequest>,
) -> Result<Json<StepUpSessionResponse>, Response> {
    let totp_code = payload.totp_code.trim();
    if totp_code.len() != 6 || !totp_code.chars().all(|digit| digit.is_ascii_digit()) {
        return Err((
            StatusCode::BAD_REQUEST,
            "MFA code must contain exactly 6 digits",
        )
            .into_response());
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to issue step-up session: {error}"),
            )
                .into_response()
        })?
        .as_secs();
    let expires_at = now.saturating_add(20 * 60);
    Ok(Json(StepUpSessionResponse {
        mfa_session_token: format!("mfa.{}.{}", claims.subject, Uuid::new_v4().simple()),
        expires_at,
    }))
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

    flush_auth_state(&state).await;
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

    flush_auth_state(&state).await;
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

    flush_auth_state(&state).await;
    Ok(Json(ConsumeRecoveryCodeResponse { token }))
}

pub(crate) async fn list_users_handler(
    State(state): State<crate::AppState>,
    Extension(claims): Extension<AuthClaims>,
) -> Result<Json<Vec<IamUserSummaryResponse>>, Response> {
    let auth_manager = Arc::clone(&state.auth_manager);
    let engine = state.runtime.load().engine.clone();
    let actor = claims.subject.clone();

    let users = tokio::task::spawn_blocking(move || auth_manager.list_users(&engine))
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to join user listing task: {error}"),
            )
                .into_response()
        })?
        .map_err(|error| {
            state.iam_audit_log.record(
                actor.clone(),
                "user.list",
                None,
                None,
                "error",
                error.to_string(),
            );
            string_error_to_response(error)
        })?;

    state.iam_audit_log.record(
        actor,
        "user.list",
        None,
        None,
        "ok",
        format!("{} users", users.len()),
    );
    Ok(Json(users))
}

pub(crate) async fn update_user_handler(
    State(state): State<crate::AppState>,
    Extension(claims): Extension<AuthClaims>,
    axum::extract::Path(username): axum::extract::Path<String>,
    Json(payload): Json<UpdateUserRequest>,
) -> Result<Json<IamUserSummaryResponse>, Response> {
    let auth_manager = Arc::clone(&state.auth_manager);
    let engine = state.runtime.load().engine.clone();
    let actor = claims.subject.clone();
    let target = username.clone();
    let action = if payload.disabled == Some(true) {
        "user.disable"
    } else if payload.disabled == Some(false) {
        "user.enable"
    } else {
        "user.update"
    };
    let detail = serde_json::to_string(&payload).unwrap_or_default();
    let update = user_update_from_request(payload);
    let actor_for_call = actor.clone();
    let target_for_call = target.clone();

    let summary = tokio::task::spawn_blocking(move || {
        auth_manager.update_user(&engine, &actor_for_call, &target_for_call, update)
    })
    .await
    .map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to join user update task: {error}"),
        )
            .into_response()
    })?
    .map_err(|error| {
        state.iam_audit_log.record(
            actor.clone(),
            action,
            Some(target.clone()),
            None,
            "error",
            error.to_string(),
        );
        string_error_to_response(error)
    })?;

    state
        .iam_audit_log
        .record(actor, action, Some(target), None, "ok", detail);
    flush_auth_state(&state).await;
    Ok(Json(summary))
}

pub(crate) async fn delete_user_handler(
    State(state): State<crate::AppState>,
    Extension(claims): Extension<AuthClaims>,
    axum::extract::Path(username): axum::extract::Path<String>,
) -> Result<StatusCode, Response> {
    let auth_manager = Arc::clone(&state.auth_manager);
    let engine = state.runtime.load().engine.clone();
    let actor = claims.subject.clone();
    let target = username.clone();
    let actor_for_call = actor.clone();
    let target_for_call = target.clone();

    tokio::task::spawn_blocking(move || {
        auth_manager.delete_user(&engine, &actor_for_call, &target_for_call)
    })
    .await
    .map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to join user delete task: {error}"),
        )
            .into_response()
    })?
    .map_err(|error| {
        state.iam_audit_log.record(
            actor.clone(),
            "user.delete",
            Some(target.clone()),
            None,
            "error",
            error.to_string(),
        );
        string_error_to_response(error)
    })?;

    state.iam_audit_log.record(
        actor,
        "user.delete",
        Some(target),
        None,
        "ok",
        String::new(),
    );
    flush_auth_state(&state).await;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn list_groups_handler(
    State(state): State<crate::AppState>,
    Extension(claims): Extension<AuthClaims>,
) -> Result<Json<Vec<IamGroupSummaryResponse>>, Response> {
    let auth_manager = Arc::clone(&state.auth_manager);
    let engine = state.runtime.load().engine.clone();
    let actor = claims.subject.clone();

    let groups = tokio::task::spawn_blocking(move || auth_manager.list_groups(&engine))
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to join group listing task: {error}"),
            )
                .into_response()
        })?
        .map_err(|error| {
            state.iam_audit_log.record(
                actor.clone(),
                "group.list",
                None,
                None,
                "error",
                error.to_string(),
            );
            string_error_to_response(error)
        })?;

    state.iam_audit_log.record(
        actor,
        "group.list",
        None,
        None,
        "ok",
        format!("{} groups", groups.len()),
    );
    Ok(Json(groups))
}

pub(crate) async fn upsert_group_handler(
    State(state): State<crate::AppState>,
    Extension(claims): Extension<AuthClaims>,
    Json(payload): Json<UpsertGroupRequest>,
) -> Result<Json<IamGroupSummaryResponse>, Response> {
    let auth_manager = Arc::clone(&state.auth_manager);
    let engine = state.runtime.load().engine.clone();
    let actor = claims.subject.clone();
    let group_name = payload.name.clone();
    let detail = format!(
        "roles={} scopes={}",
        payload.roles.join(","),
        payload.scopes.join(",")
    );
    let input = group_input_from_request(payload);

    let summary = tokio::task::spawn_blocking(move || auth_manager.upsert_group(&engine, input))
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to join group upsert task: {error}"),
            )
                .into_response()
        })?
        .map_err(|error| {
            state.iam_audit_log.record(
                actor.clone(),
                "group.upsert",
                None,
                Some(group_name.clone()),
                "error",
                error.to_string(),
            );
            string_error_to_response(error)
        })?;

    state
        .iam_audit_log
        .record(actor, "group.upsert", None, Some(group_name), "ok", detail);
    flush_auth_state(&state).await;
    Ok(Json(summary))
}

pub(crate) async fn delete_group_handler(
    State(state): State<crate::AppState>,
    Extension(claims): Extension<AuthClaims>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Result<StatusCode, Response> {
    let auth_manager = Arc::clone(&state.auth_manager);
    let engine = state.runtime.load().engine.clone();
    let actor = claims.subject.clone();
    let group_name = name.clone();

    tokio::task::spawn_blocking(move || auth_manager.delete_group(&engine, &group_name))
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to join group delete task: {error}"),
            )
                .into_response()
        })?
        .map_err(|error| {
            state.iam_audit_log.record(
                actor.clone(),
                "group.delete",
                None,
                Some(name.clone()),
                "error",
                error.to_string(),
            );
            string_error_to_response(error)
        })?;

    state
        .iam_audit_log
        .record(actor, "group.delete", None, Some(name), "ok", String::new());
    flush_auth_state(&state).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuditLogQuery {
    #[serde(default)]
    pub(crate) user: Option<String>,
    #[serde(default)]
    pub(crate) lines: Option<usize>,
}

pub(crate) async fn audit_log_handler(
    State(state): State<crate::AppState>,
    axum::extract::Query(query): axum::extract::Query<AuditLogQuery>,
) -> Json<Vec<crate::iam_audit::IamAuditEntry>> {
    let lines = query.lines.unwrap_or(crate::iam_audit::DEFAULT_TAIL);
    let user = query.user.as_deref();
    Json(state.iam_audit_log.snapshot(user, lines))
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

fn map_registration_claims(
    claims: AuthnRegistrationTokenClaims,
) -> RegistrationTokenClaimsResponse {
    RegistrationTokenClaimsResponse {
        subject: claims.subject,
        roles: claims.roles,
        scopes: claims.scopes,
        expires_at: claims.expires_at,
    }
}

fn map_staged_user_session(session: AuthnStagedUserSession) -> StagedUserSessionResponse {
    StagedUserSessionResponse {
        session_id: session.session_id,
        username: session.username,
        provisioning_uri: session.provisioning_uri,
        roles: session.roles,
        scopes: session.scopes,
        expires_at: session.expires_at,
    }
}

fn map_staged_login_session(session: AuthnStagedLoginSession) -> StagedLoginSessionResponse {
    StagedLoginSessionResponse {
        session_id: session.session_id,
        username: session.username,
        expires_at: session.expires_at,
    }
}

fn map_user_summary(summary: AuthnUserSummary) -> IamUserSummaryResponse {
    IamUserSummaryResponse {
        username: summary.username,
        first_name: summary.first_name,
        last_name: summary.last_name,
        roles: summary.roles,
        scopes: summary.scopes,
        groups: summary.groups,
        disabled_at: summary.disabled_at,
        created_at: summary.created_at,
        last_login_at: summary.last_login_at,
    }
}

fn map_group_summary(summary: AuthnGroupSummary) -> IamGroupSummaryResponse {
    IamGroupSummaryResponse {
        name: summary.name,
        description: summary.description,
        roles: summary.roles,
        scopes: summary.scopes,
        member_count: summary.member_count,
        created_at: summary.created_at,
        updated_at: summary.updated_at,
    }
}

pub(crate) fn user_update_from_request(request: UpdateUserRequest) -> AuthnUserUpdate {
    AuthnUserUpdate {
        add_groups: request.add_groups,
        remove_groups: request.remove_groups,
        add_roles: request.add_roles,
        remove_roles: request.remove_roles,
        add_scopes: request.add_scopes,
        remove_scopes: request.remove_scopes,
        disabled: request.disabled,
    }
}

pub(crate) fn group_input_from_request(request: UpsertGroupRequest) -> AuthnGroupInput {
    AuthnGroupInput {
        name: request.name,
        description: request.description,
        roles: request.roles,
        scopes: request.scopes,
    }
}

fn map_auth_session(session: AuthnSession) -> FinalizeEnrollmentResponse {
    FinalizeEnrollmentResponse {
        token: session.token,
        username: session.username,
        roles: session.roles,
        scopes: session.scopes,
    }
}

/// Flush the auth-state directory to S3 after any mutating auth operation.
/// No-op when the `s3-persistence` feature is disabled or no backend is configured.
async fn flush_auth_state(state: &crate::AppState) {
    #[cfg(feature = "s3-persistence")]
    if let Some(backend) = state.s3_backend.as_deref() {
        let auth_dir = auth_state_dir(&state.manifest_path);
        match tokio::time::timeout(AUTH_STATE_FLUSH_TIMEOUT, backend.flush_path(&auth_dir)).await {
            Ok(Ok(())) => {
                tracing::info!(path = %auth_dir.display(), "flushed auth state to S3");
            }
            Ok(Err(error)) => {
                tracing::warn!(error = %error, "failed to flush auth state to S3");
            }
            Err(_) => {
                tracing::warn!(
                    timeout_ms = AUTH_STATE_FLUSH_TIMEOUT.as_millis(),
                    "timed out flushing auth state to S3"
                );
            }
        }
    }
    #[cfg(not(feature = "s3-persistence"))]
    let _ = state;
}

fn string_error_to_response(error: anyhow::Error) -> Response {
    let message = error.to_string();
    let status = if message.contains("must not be empty")
        || message.contains("must match")
        || message.contains("invalid")
        || message.contains("expired")
        || message.contains("already")
        || message.contains("between 1 and")
    {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };

    (status, message).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "experimental")]
    use crate::store::CoreStore;

    fn fresh_claims(subject: &str, roles: &[&str]) -> AuthClaims {
        AuthClaims {
            subject: subject.to_owned(),
            roles: roles.iter().map(|r| (*r).to_owned()).collect(),
            scopes: Vec::new(),
        }
    }

    #[cfg(feature = "experimental")]
    fn token_hash_hex(token: &str) -> String {
        use sha2::Digest;
        let digest = sha2::Sha256::digest(token.as_bytes());
        hex::encode(digest)
    }

    #[test]
    fn resolve_jwt_secret_prefers_env_then_persists_a_random_fallback() {
        // No other test in this crate touches TACHYON_AUTH_JWT_SECRET, so the
        // sequential set/remove below is race-free against the rest of the suite.
        let dir = tempfile::tempdir().expect("temp state dir");

        // 1. An explicit operator-provided secret always wins.
        std::env::set_var(JWT_SECRET_ENV, "explicit-operator-secret");
        assert_eq!(resolve_jwt_secret(dir.path()), "explicit-operator-secret");

        // 2. With no secret configured, a random 256-bit value is generated and
        //    persisted — never the old hard-coded constant.
        std::env::remove_var(JWT_SECRET_ENV);
        let generated = resolve_jwt_secret(dir.path());
        assert_eq!(generated.len(), 64);
        assert!(generated.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(generated, "tachyon-dev-secret");
        assert!(dir.path().join(JWT_SECRET_FILE).exists());

        // 3. The persisted secret is reused on the next resolution, so a single
        //    node keeps a stable secret across restarts.
        assert_eq!(resolve_jwt_secret(dir.path()), generated);
    }

    #[test]
    fn cache_round_trips_token_method_path_decision() {
        let cache = AuthDecisionCache::new();
        let claims = fresh_claims("alice", &["admin"]);
        cache.put("tok-1", "GET", "/api/x", claims.clone());
        let got = cache.get("tok-1", "GET", "/api/x").expect("cached");
        assert_eq!(got.subject, "alice");
        // Different method/path is a cache miss.
        assert!(cache.get("tok-1", "POST", "/api/x").is_none());
        assert!(cache.get("tok-1", "GET", "/api/y").is_none());
    }

    #[test]
    fn invalidate_token_evicts_only_matching_entries() {
        let cache = AuthDecisionCache::new();
        cache.put("tok-1", "GET", "/api/x", fresh_claims("alice", &["admin"]));
        cache.put("tok-2", "GET", "/api/x", fresh_claims("bob", &["user"]));

        let target_hash = {
            use sha2::Digest;
            let mut buf = [0u8; 32];
            buf.copy_from_slice(sha2::Sha256::digest(b"tok-1").as_slice());
            buf
        };
        cache.invalidate_token(&target_hash);

        // Wait for moka's lazy invalidation queue to drain.
        cache.inner.run_pending_tasks();
        assert!(cache.get("tok-1", "GET", "/api/x").is_none());
        assert!(cache.get("tok-2", "GET", "/api/x").is_some());
    }

    #[test]
    fn invalidate_subject_evicts_every_token_for_user() {
        let cache = AuthDecisionCache::new();
        cache.put("tok-a1", "GET", "/api/x", fresh_claims("alice", &["admin"]));
        cache.put("tok-a2", "POST", "/api/y", fresh_claims("alice", &["user"]));
        cache.put("tok-b1", "GET", "/api/x", fresh_claims("bob", &["user"]));

        cache.invalidate_subject("alice");
        cache.inner.run_pending_tasks();

        assert!(cache.get("tok-a1", "GET", "/api/x").is_none());
        assert!(cache.get("tok-a2", "POST", "/api/y").is_none());
        // Bob's entry untouched.
        assert!(cache.get("tok-b1", "GET", "/api/x").is_some());
    }

    #[test]
    fn default_pat_ttl_is_thirty_days() {
        assert_eq!(default_pat_ttl_days(), 30);
    }

    #[test]
    fn auth_state_dir_prefers_explicit_env_then_manifest_parent() {
        let manifest = Path::new("/tmp/tachyon/manifest.json");
        std::env::set_var(AUTH_STATE_DIR_ENV, "/app/auth-state");
        assert_eq!(auth_state_dir(manifest), PathBuf::from("/app/auth-state"));

        std::env::remove_var(AUTH_STATE_DIR_ENV);
        assert_eq!(
            auth_state_dir(manifest),
            Path::new("/tmp/tachyon/auth-state")
        );
        assert_eq!(
            auth_state_dir(Path::new("manifest.json")),
            PathBuf::from("auth-state")
        );
    }

    #[test]
    fn manager_exposes_its_own_decision_cache() {
        let manager = AuthManager {
            authn_module_name: "missing-authn".to_owned(),
            authz_module_name: "missing-authz".to_owned(),
            state_dir: PathBuf::from("unused-auth-state"),
            jwt_secret: "secret".to_owned(),
            decision_cache: AuthDecisionCache::new(),
        };
        manager.decision_cache.put(
            "tok",
            "GET",
            "/admin/status",
            fresh_claims("alice", &["admin"]),
        );

        let cached = manager
            .decision_cache()
            .get("tok", "GET", "/admin/status")
            .expect("manager should return the configured cache");
        assert_eq!(cached.subject, "alice");
    }

    #[test]
    fn auth_manager_methods_fail_when_guest_module_is_missing() {
        let dir = tempdir();
        let manager = AuthManager {
            authn_module_name: "missing-authn".to_owned(),
            authz_module_name: "missing-authz".to_owned(),
            state_dir: dir.path().join("auth-state"),
            jwt_secret: "secret".to_owned(),
            decision_cache: AuthDecisionCache::new(),
        };
        let engine = Engine::default();

        assert!(manager
            .validate_registration_token(&engine, "invalid-registration-token")
            .is_err());
        assert!(manager
            .stage_user(
                &engine,
                StageSignupRequest {
                    token: "invalid-registration-token".to_owned(),
                    first_name: "Alice".to_owned(),
                    last_name: "Mesh".to_owned(),
                    username: "alice".to_owned(),
                    password: "correct horse battery staple".to_owned(),
                },
            )
            .is_err());
        assert!(manager
            .finalize_enrollment(&engine, "missing-session", "123456")
            .is_err());
        assert!(manager
            .stage_login(&engine, "alice", "wrong-password")
            .is_err());
        assert!(manager
            .finalize_login(&engine, "missing-session", "123456")
            .is_err());
        assert!(manager.generate_recovery_codes(&engine, "alice").is_err());
        assert!(manager
            .consume_recovery_code(&engine, "alice", "bad-code")
            .is_err());
        assert!(manager
            .issue_pat(&engine, "alice", "laptop", &["scope:a".to_owned()], 30)
            .is_err());
        assert!(manager.list_users(&engine).is_err());
        assert!(manager
            .update_user(
                &engine,
                "admin",
                "alice",
                AuthnUserUpdate {
                    add_groups: None,
                    remove_groups: None,
                    add_roles: None,
                    remove_roles: None,
                    add_scopes: None,
                    remove_scopes: None,
                    disabled: None,
                },
            )
            .is_err());
        assert!(manager.delete_user(&engine, "admin", "alice").is_err());
        assert!(manager.list_groups(&engine).is_err());
        assert!(manager
            .upsert_group(
                &engine,
                AuthnGroupInput {
                    name: "ops".to_owned(),
                    description: String::new(),
                    roles: Vec::new(),
                    scopes: Vec::new(),
                },
            )
            .is_err());
        assert!(manager.delete_group(&engine, "ops").is_err());
        assert!(manager
            .authorize(&engine, &fresh_claims("alice", &[]), "GET", "/admin/status")
            .is_err());
    }

    #[test]
    fn authn_guest_can_stage_user_with_preopened_mutable_state_dir() {
        if let Err(error) = crate::resolve_guest_module_path("system-faas-authn") {
            eprintln!("SKIP: system-faas-authn artifact not present: {error}");
            return;
        }

        let dir = tempdir();
        let manager = test_auth_manager_with_modules(
            "system-faas-authn",
            "system-faas-authz",
            dir.path().join("auth-state"),
        );
        let engine = Engine::default();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_secs();
        let token = issue_registration_token_for_auth_test(
            "test-secret",
            "invite:alice",
            &["admin"],
            &["scope:admin"],
            now,
            now + 600,
        );

        let staged = manager
            .stage_user(
                &engine,
                StageSignupRequest {
                    token,
                    first_name: "Alice".to_owned(),
                    last_name: "Mesh".to_owned(),
                    username: format!("alice-{}", Uuid::new_v4().simple()),
                    password: "correct horse battery staple".to_owned(),
                },
            )
            .expect("authn guest should be able to write pending enrollment state");

        assert_eq!(staged.roles, vec!["admin"]);
        assert_eq!(staged.scopes, vec!["scope:admin"]);
        assert!(staged.provisioning_uri.starts_with("otpauth://totp/"));
    }

    #[test]
    fn apply_authz_purge_rejects_bad_token_hash_and_evicts_subject_events() {
        let cache = AuthDecisionCache::new();
        cache.put("tok-a", "GET", "/api/x", fresh_claims("alice", &["admin"]));
        cache.put("tok-b", "GET", "/api/x", fresh_claims("bob", &["user"]));

        let bad_hex = AuthzPurgeEvent::Token {
            token_hash: "not-hex".to_owned(),
            ts_ms: 1,
        };
        assert!(apply_authz_purge(&cache, &bad_hex)
            .expect_err("invalid hex must fail")
            .to_string()
            .contains("hex"));

        let short_hash = AuthzPurgeEvent::Token {
            token_hash: "abcd".to_owned(),
            ts_ms: 1,
        };
        assert!(apply_authz_purge(&cache, &short_hash)
            .expect_err("short hashes must fail")
            .to_string()
            .contains("32 bytes"));

        apply_authz_purge(
            &cache,
            &AuthzPurgeEvent::Role {
                user_id: "alice".to_owned(),
                ts_ms: 2,
            },
        )
        .expect("role event should apply");
        cache.inner.run_pending_tasks();
        assert!(cache.get("tok-a", "GET", "/api/x").is_none());
        assert!(cache.get("tok-b", "GET", "/api/x").is_some());

        apply_authz_purge(
            &cache,
            &AuthzPurgeEvent::UserBan {
                user_id: "bob".to_owned(),
                ts_ms: 3,
            },
        )
        .expect("ban event should apply");
        cache.inner.run_pending_tasks();
        assert!(cache.get("tok-b", "GET", "/api/x").is_none());
    }

    #[test]
    fn bearer_token_accepts_only_nonempty_bearer_scheme() {
        let mut headers = HeaderMap::new();
        assert!(matches!(
            bearer_token(&headers),
            Err(AuthFailure::Unauthorized(message)) if message.contains("missing")
        ));

        headers.insert(AUTHORIZATION, "Token abc".parse().expect("header"));
        assert!(matches!(
            bearer_token(&headers),
            Err(AuthFailure::Unauthorized(message)) if message.contains("Bearer")
        ));

        headers.insert(AUTHORIZATION, "Bearer    ".parse().expect("header"));
        assert!(matches!(
            bearer_token(&headers),
            Err(AuthFailure::Unauthorized(message)) if message.contains("Bearer")
        ));

        headers.insert(AUTHORIZATION, "Bearer abc123   ".parse().expect("header"));
        assert_eq!(bearer_token(&headers).expect("token"), "abc123");
    }

    #[tokio::test]
    async fn issue_step_up_session_requires_six_digit_code_and_twenty_minute_ttl() {
        let claims = fresh_claims("alice", &["admin"]);
        for bad_code in ["12345", "1234567", "12a456", ""] {
            let response = issue_step_up_session_handler(
                Extension(claims.clone()),
                Json(StepUpSessionRequest {
                    totp_code: bad_code.to_owned(),
                }),
            )
            .await
            .expect_err("invalid MFA code should fail");
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }

        let before = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_secs();
        let response = issue_step_up_session_handler(
            Extension(claims),
            Json(StepUpSessionRequest {
                totp_code: " 123456 ".to_owned(),
            }),
        )
        .await
        .expect("valid MFA code should issue a session")
        .0;
        let after = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_secs();

        assert!(response.mfa_session_token.starts_with("mfa.alice."));
        assert!(response.expires_at >= before + 20 * 60);
        assert!(response.expires_at <= after + 20 * 60);
    }

    #[test]
    fn string_error_to_response_maps_known_client_errors_to_bad_request() {
        for message in [
            "name must not be empty",
            "username must match allowed pattern",
            "invalid token",
            "token expired",
            "user already exists",
            "ttl must be between 1 and 365",
        ] {
            let response = string_error_to_response(anyhow!(message));
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{message}");
        }

        let response = string_error_to_response(anyhow!("storage backend unavailable"));
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    fn issue_registration_token_for_auth_test(
        secret: &str,
        subject: &str,
        roles: &[&str],
        scopes: &[&str],
        issued_at: u64,
        expires_at: u64,
    ) -> String {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
        use hmac::{Hmac, KeyInit, Mac};
        use serde_json::json;
        type HmacSha256 = Hmac<sha2::Sha256>;

        let header = json!({
            "alg": "HS256",
            "typ": "JWT",
        });
        let payload = json!({
            "sub": subject,
            "iat": issued_at,
            "exp": expires_at,
            "token_use": "registration",
            "invite_roles": roles,
            "invite_scopes": scopes,
        });
        let encoded_header =
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).expect("header should encode"));
        let encoded_payload =
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).expect("payload should encode"));
        let signing_input = format!("{encoded_header}.{encoded_payload}");
        let mut mac =
            HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC should initialize");
        mac.update(signing_input.as_bytes());
        let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        format!("{signing_input}.{signature}")
    }

    #[cfg(feature = "experimental")]
    #[test]
    fn enqueue_round_trips_through_outbox_and_apply_evicts() {
        let dir = tempdir();
        let db_path = dir.path().join("auth-cache-test.redb");
        let store = CoreStore::open(&db_path).expect("redb open");
        let cache = AuthDecisionCache::new();
        cache.put(
            "tok-rev",
            "GET",
            "/api/x",
            fresh_claims("carol", &["admin"]),
        );
        assert!(cache.get("tok-rev", "GET", "/api/x").is_some());

        let event = AuthzPurgeEvent::Token {
            token_hash: token_hash_hex("tok-rev"),
            ts_ms: 1_700_000_000_000,
        };
        event.enqueue(&store).expect("enqueue");

        let rows = store
            .peek_outbox(crate::store::CoreStoreBucket::AuthzPurgeOutbox, 16)
            .expect("peek");
        assert_eq!(rows.len(), 1);
        let parsed: AuthzPurgeEvent =
            serde_json::from_slice(&rows[0].1).expect("payload parses back");
        assert_eq!(parsed, event);

        apply_authz_purge(&cache, &parsed).expect("apply");
        cache.inner.run_pending_tasks();
        assert!(cache.get("tok-rev", "GET", "/api/x").is_none());
    }

    // Tiny inline tempdir helper. Keeps the test file from pulling in `tempfile`.
    struct TempDir {
        path: std::path::PathBuf,
    }
    impl TempDir {
        fn path(&self) -> &Path {
            &self.path
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
    fn tempdir() -> TempDir {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("core-host-auth-test-{pid}-{nanos}"));
        std::fs::create_dir_all(&path).expect("create tempdir");
        TempDir { path }
    }
}

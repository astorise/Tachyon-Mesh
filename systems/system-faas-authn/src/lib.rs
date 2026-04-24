mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/authn.wit",
        world: "authn-guest",
    });

    export!(Component);
}

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use bindings::exports::tachyon::identity::authn::{
    AuthSession, AuthnError, IdentityPayload, RegistrationTokenClaims, SignupProfile,
    StagedUserSession,
};
use hmac::{Hmac, KeyInit, Mac};
use rand::distr::{Alphanumeric, SampleString};
use serde::{Deserialize, Serialize};
use sha1::{Digest as _, Sha1};
use sha2::{Digest as _, Sha256};
use std::{
    env, fs,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const DEFAULT_JWT_SECRET: &str = "tachyon-dev-secret";
const JWT_SECRET_ENV: &str = "TACHYON_AUTH_JWT_SECRET";
const AUTH_STATE_DIR_ENV: &str = "TACHYON_AUTH_STATE_DIR";
const PAT_PREFIX: &str = "tpat_";
const PAT_TOKEN_CHARS: usize = 48;
const MAX_PAT_TTL_DAYS: u32 = 365;
const RECOVERY_CODE_COUNT: usize = 10;
const RECOVERY_SESSION_TTL_SECONDS: u64 = 300;
const REGISTRATION_TOKEN_USE: &str = "registration";
const REGISTRATION_TOKEN_TTL_SECONDS: u64 = 24 * 60 * 60;
const AUTH_SESSION_TTL_SECONDS: u64 = 24 * 60 * 60;
const TOTP_SECRET_BYTES: usize = 20;
const TOTP_PERIOD_SECONDS: u64 = 30;
const TOTP_ALLOWED_SKEW_STEPS: i64 = 1;
const TOTP_DIGITS: u32 = 6;
const TOTP_ISSUER: &str = "Tachyon Mesh";

type HmacSha256 = Hmac<Sha256>;

struct Component;

#[derive(Debug, Deserialize)]
struct JwtHeader {
    alg: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TokenPayload {
    #[serde(rename = "sub")]
    subject: String,
    #[serde(default)]
    roles: Vec<String>,
    #[serde(default)]
    scopes: Vec<String>,
    #[serde(default)]
    exp: Option<u64>,
    #[serde(default)]
    iat: Option<u64>,
    #[serde(default)]
    token_use: Option<String>,
    #[serde(default)]
    invite_roles: Vec<String>,
    #[serde(default)]
    invite_scopes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersonalAccessTokenRecord {
    owner: String,
    name: String,
    hash: String,
    scopes: Vec<String>,
    created_at: u64,
    expires_at: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct UserSecurityRecord {
    #[serde(default)]
    recovery_hashes: Vec<String>,
    #[serde(default)]
    pats: Vec<PersonalAccessTokenRecord>,
    #[serde(default)]
    profile: Option<UserProfileRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UserProfileRecord {
    first_name: String,
    last_name: String,
    username: String,
    password_hash: String,
    totp_secret: String,
    roles: Vec<String>,
    scopes: Vec<String>,
    created_at: u64,
    activated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingEnrollmentRecord {
    session_id: String,
    registration_token_hash: String,
    invite_subject: String,
    first_name: String,
    last_name: String,
    username: String,
    password_hash: String,
    totp_secret: String,
    roles: Vec<String>,
    scopes: Vec<String>,
    created_at: u64,
    expires_at: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ConsumedRegistrationTokenStore {
    #[serde(default)]
    hashes: Vec<String>,
}

impl bindings::exports::tachyon::identity::authn::Guest for Component {
    fn validate_token(token: String) -> Result<IdentityPayload, AuthnError> {
        validate_identity_token(&token)
    }

    fn validate_registration_token(token: String) -> Result<RegistrationTokenClaims, String> {
        validate_registration_token_claims(&token)
    }

    fn stage_user(token: String, profile: SignupProfile) -> Result<StagedUserSession, String> {
        stage_pending_user(&token, profile)
    }

    fn finalize_enrollment(session_id: String, totp_code: String) -> Result<AuthSession, String> {
        finalize_pending_enrollment(&session_id, &totp_code)
    }

    fn issue_pat(
        subject: String,
        name: String,
        scopes: Vec<String>,
        ttl_days: u32,
    ) -> Result<String, String> {
        issue_pat_token(&subject, &name, &scopes, ttl_days)
    }

    fn generate_recovery_codes(username: String) -> Result<Vec<String>, String> {
        let username = normalize_username(&username)?;
        let mut record = load_user_record(&username)?;
        let mut codes = Vec::with_capacity(RECOVERY_CODE_COUNT);
        let mut hashes = Vec::with_capacity(RECOVERY_CODE_COUNT);

        for _ in 0..RECOVERY_CODE_COUNT {
            let code = generate_recovery_code();
            hashes.push(hash_recovery_code(&username, &code));
            codes.push(code);
        }

        record.recovery_hashes = hashes;
        save_user_record(&username, &record)?;
        Ok(codes)
    }

    fn consume_recovery_code(username: String, code: String) -> Result<String, String> {
        let username = normalize_username(&username)?;
        let normalized_code = normalize_recovery_code(&code)?;
        let mut record = load_user_record(&username)?;
        let expected_hash = hash_recovery_code(&username, &normalized_code);

        let Some(index) = record
            .recovery_hashes
            .iter()
            .position(|candidate| candidate == &expected_hash)
        else {
            return Err("recovery code is invalid or already used".to_owned());
        };

        record.recovery_hashes.remove(index);
        save_user_record(&username, &record)?;
        issue_jwt(
            &username,
            &[String::from("admin"), String::from("recovery")],
            &[],
            Duration::from_secs(RECOVERY_SESSION_TTL_SECONDS),
        )
    }
}

fn validate_identity_token(token: &str) -> Result<IdentityPayload, AuthnError> {
    if token.starts_with(PAT_PREFIX) {
        return resolve_pat_identity(token);
    }

    let payload = verify_hs256_token(token)?;
    Ok(IdentityPayload {
        subject: payload.subject,
        roles: normalize_roles(payload.roles),
        scopes: normalize_scopes(payload.scopes),
    })
}

fn validate_registration_token_claims(token: &str) -> Result<RegistrationTokenClaims, String> {
    let payload = verify_hs256_token(token).map_err(map_authn_error_to_string)?;
    validate_registration_payload(&payload, token)
}

fn validate_registration_payload(
    payload: &TokenPayload,
    token: &str,
) -> Result<RegistrationTokenClaims, String> {
    let now = unix_timestamp_seconds().map_err(|error| error.to_string())?;
    let issued_at = payload
        .iat
        .ok_or_else(|| "registration token must include `iat`".to_owned())?;
    let expires_at = payload
        .exp
        .ok_or_else(|| "registration token must include `exp`".to_owned())?;

    if payload.token_use.as_deref() != Some(REGISTRATION_TOKEN_USE) {
        return Err("registration token must declare `token_use=registration`".to_owned());
    }
    if expires_at <= issued_at {
        return Err("registration token expiry must be after issuance".to_owned());
    }
    if expires_at.saturating_sub(issued_at) > REGISTRATION_TOKEN_TTL_SECONDS {
        return Err("registration token TTL must not exceed 24 hours".to_owned());
    }
    if now >= expires_at {
        return Err("registration token has expired".to_owned());
    }

    let token_hash = hash_registration_token(token);
    if registration_token_is_consumed(&token_hash)? {
        return Err("registration token has already been consumed".to_owned());
    }

    Ok(RegistrationTokenClaims {
        subject: payload.subject.clone(),
        roles: normalize_roles(payload.invite_roles.clone()),
        scopes: normalize_scopes(payload.invite_scopes.clone()),
        expires_at,
    })
}

fn stage_pending_user(token: &str, profile: SignupProfile) -> Result<StagedUserSession, String> {
    let claims = validate_registration_token_claims(token)?;
    if claims.roles.is_empty() && claims.scopes.is_empty() {
        return Err("registration token does not grant any roles or scopes".to_owned());
    }

    let username = normalize_username(&profile.username)?;
    prune_expired_pending_enrollments()?;
    ensure_user_is_available(&username)?;

    let now = unix_timestamp_seconds().map_err(|error| error.to_string())?;
    let pending = PendingEnrollmentRecord {
        session_id: generate_pending_enrollment_id(),
        registration_token_hash: hash_registration_token(token),
        invite_subject: claims.subject.clone(),
        first_name: normalize_person_name(&profile.first_name, "first name")?,
        last_name: normalize_person_name(&profile.last_name, "last name")?,
        username: username.clone(),
        password_hash: hash_user_password(&username, &profile.password)?,
        totp_secret: generate_totp_secret(),
        roles: claims.roles.clone(),
        scopes: claims.scopes.clone(),
        created_at: now,
        expires_at: claims.expires_at,
    };
    save_pending_enrollment(&pending)?;

    Ok(StagedUserSession {
        session_id: pending.session_id.clone(),
        username,
        provisioning_uri: build_totp_provisioning_uri(&pending.username, &pending.totp_secret),
        roles: claims.roles,
        scopes: claims.scopes,
        expires_at: pending.expires_at,
    })
}

fn finalize_pending_enrollment(session_id: &str, totp_code: &str) -> Result<AuthSession, String> {
    let session_id = normalize_session_id(session_id)?;
    let pending = load_pending_enrollment(&session_id)?;
    let now = unix_timestamp_seconds().map_err(|error| error.to_string())?;

    if now >= pending.expires_at {
        delete_pending_enrollment(&session_id)?;
        return Err("enrollment session has expired".to_owned());
    }

    if registration_token_is_consumed(&pending.registration_token_hash)? {
        delete_pending_enrollment(&session_id)?;
        return Err("registration token has already been consumed".to_owned());
    }

    verify_totp_code(&pending.totp_secret, totp_code, now)?;

    let mut record = load_user_record(&pending.username)?;
    if record.profile.is_some() {
        delete_pending_enrollment(&session_id)?;
        return Err("username is already enrolled".to_owned());
    }

    record.profile = Some(UserProfileRecord {
        first_name: pending.first_name.clone(),
        last_name: pending.last_name.clone(),
        username: pending.username.clone(),
        password_hash: pending.password_hash.clone(),
        totp_secret: pending.totp_secret.clone(),
        roles: pending.roles.clone(),
        scopes: pending.scopes.clone(),
        created_at: pending.created_at,
        activated_at: now,
    });
    save_user_record(&pending.username, &record)?;
    consume_registration_token(&pending.registration_token_hash)?;
    delete_pending_enrollment(&session_id)?;

    let token = issue_jwt(
        &pending.username,
        &pending.roles,
        &pending.scopes,
        Duration::from_secs(AUTH_SESSION_TTL_SECONDS),
    )?;

    Ok(AuthSession {
        token,
        username: pending.username,
        roles: pending.roles,
        scopes: pending.scopes,
    })
}

fn resolve_pat_identity(token: &str) -> Result<IdentityPayload, AuthnError> {
    let state_dir = state_root_dir();
    fs::create_dir_all(&state_dir).map_err(|error| {
        AuthnError::InternalError(format!(
            "failed to initialize auth state directory {}: {error}",
            state_dir.display()
        ))
    })?;
    let now = unix_timestamp_seconds().map_err(|error| {
        AuthnError::InternalError(format!("failed to read system clock: {error}"))
    })?;
    let token_hash = hash_pat_token(token);

    for entry in fs::read_dir(&state_dir).map_err(|error| {
        AuthnError::InternalError(format!(
            "failed to enumerate auth state directory {}: {error}",
            state_dir.display()
        ))
    })? {
        let entry = entry.map_err(|error| {
            AuthnError::InternalError(format!("failed to inspect auth state entry: {error}"))
        })?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }

        let raw = fs::read(&path).map_err(|error| {
            AuthnError::InternalError(format!(
                "failed to read auth state {}: {error}",
                path.display()
            ))
        })?;
        let mut record: UserSecurityRecord = serde_json::from_slice(&raw).map_err(|error| {
            AuthnError::InternalError(format!(
                "failed to decode auth state {}: {error}",
                path.display()
            ))
        })?;

        let original_len = record.pats.len();
        record.pats.retain(|pat| pat.expires_at > now);
        if record.pats.len() != original_len {
            let payload = serde_json::to_vec_pretty(&record).map_err(|error| {
                AuthnError::InternalError(format!("failed to encode auth state: {error}"))
            })?;
            fs::write(&path, payload).map_err(|error| {
                AuthnError::InternalError(format!(
                    "failed to persist pruned auth state {}: {error}",
                    path.display()
                ))
            })?;
        }

        if let Some(pat) = record
            .pats
            .iter()
            .find(|candidate| candidate.hash == token_hash)
        {
            if pat.expires_at <= now {
                return Err(AuthnError::Expired);
            }
            return Ok(IdentityPayload {
                subject: pat.owner.clone(),
                roles: Vec::new(),
                scopes: normalize_scopes(pat.scopes.clone()),
            });
        }
    }

    Err(AuthnError::InvalidCredential)
}

fn issue_pat_token(
    subject: &str,
    name: &str,
    scopes: &[String],
    ttl_days: u32,
) -> Result<String, String> {
    let subject = normalize_username(subject)?;
    let name = normalize_pat_name(name)?;
    let scopes = normalize_scopes(scopes.to_vec());
    if scopes.is_empty() {
        return Err("PAT scopes must not be empty".to_owned());
    }
    if ttl_days == 0 || ttl_days > MAX_PAT_TTL_DAYS {
        return Err(format!(
            "PAT TTL must be between 1 and {MAX_PAT_TTL_DAYS} days"
        ));
    }

    let mut record = load_user_record(&subject)?;
    let now = unix_timestamp_seconds().map_err(|error| error.to_string())?;
    record.pats.retain(|pat| pat.expires_at > now);

    let token = generate_pat_token();
    let expires_at = now.saturating_add(ttl_days as u64 * 24 * 60 * 60);
    record.pats.push(PersonalAccessTokenRecord {
        owner: subject.clone(),
        name,
        hash: hash_pat_token(&token),
        scopes,
        created_at: now,
        expires_at,
    });
    save_user_record(&subject, &record)?;
    Ok(token)
}

fn map_authn_error_to_string(error: AuthnError) -> String {
    match error {
        AuthnError::Expired => "token has expired".to_owned(),
        AuthnError::InvalidCredential => {
            "token is malformed, unknown, or has an invalid signature".to_owned()
        }
        AuthnError::InternalError(message) => message,
    }
}

fn registration_token_is_consumed(token_hash: &str) -> Result<bool, String> {
    Ok(load_consumed_registration_tokens()?
        .hashes
        .contains(&token_hash.to_owned()))
}

fn consume_registration_token(token_hash: &str) -> Result<(), String> {
    let mut store = load_consumed_registration_tokens()?;
    if !store.hashes.contains(&token_hash.to_owned()) {
        store.hashes.push(token_hash.to_owned());
        store.hashes.sort();
        store.hashes.dedup();
        save_consumed_registration_tokens(&store)?;
    }
    Ok(())
}

fn ensure_user_is_available(username: &str) -> Result<(), String> {
    let user_record = load_user_record(username)?;
    if user_record.profile.is_some() {
        return Err("username is already enrolled".to_owned());
    }

    let pending_dir = pending_enrollment_dir();
    if !pending_dir.exists() {
        return Ok(());
    }

    let now = unix_timestamp_seconds().map_err(|error| error.to_string())?;
    for entry in fs::read_dir(&pending_dir).map_err(|error| {
        format!(
            "failed to enumerate pending enrollments in {}: {error}",
            pending_dir.display()
        )
    })? {
        let entry = entry.map_err(|error| {
            format!(
                "failed to inspect pending enrollment entry in {}: {error}",
                pending_dir.display()
            )
        })?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }

        let raw = fs::read(&path).map_err(|error| {
            format!(
                "failed to read pending enrollment from {}: {error}",
                path.display()
            )
        })?;
        let record: PendingEnrollmentRecord = serde_json::from_slice(&raw).map_err(|error| {
            format!(
                "failed to decode pending enrollment from {}: {error}",
                path.display()
            )
        })?;

        if record.username == username && record.expires_at > now {
            return Err("username already has a pending enrollment".to_owned());
        }
    }

    Ok(())
}

fn prune_expired_pending_enrollments() -> Result<(), String> {
    let pending_dir = pending_enrollment_dir();
    if !pending_dir.exists() {
        return Ok(());
    }

    let now = unix_timestamp_seconds().map_err(|error| error.to_string())?;
    for entry in fs::read_dir(&pending_dir).map_err(|error| {
        format!(
            "failed to enumerate pending enrollments in {}: {error}",
            pending_dir.display()
        )
    })? {
        let entry = entry.map_err(|error| {
            format!(
                "failed to inspect pending enrollment entry in {}: {error}",
                pending_dir.display()
            )
        })?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }

        let raw = fs::read(&path).map_err(|error| {
            format!(
                "failed to read pending enrollment from {}: {error}",
                path.display()
            )
        })?;
        let record: PendingEnrollmentRecord = serde_json::from_slice(&raw).map_err(|error| {
            format!(
                "failed to decode pending enrollment from {}: {error}",
                path.display()
            )
        })?;
        if record.expires_at <= now {
            fs::remove_file(&path).map_err(|error| {
                format!(
                    "failed to remove expired pending enrollment {}: {error}",
                    path.display()
                )
            })?;
        }
    }

    Ok(())
}

fn save_pending_enrollment(record: &PendingEnrollmentRecord) -> Result<(), String> {
    let path = pending_enrollment_path(&record.session_id)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create pending enrollment directory {}: {error}",
                parent.display()
            )
        })?;
    }

    let payload = serde_json::to_vec_pretty(record)
        .map_err(|error| format!("failed to encode pending enrollment: {error}"))?;
    fs::write(&path, payload).map_err(|error| {
        format!(
            "failed to persist pending enrollment to {}: {error}",
            path.display()
        )
    })
}

fn load_pending_enrollment(session_id: &str) -> Result<PendingEnrollmentRecord, String> {
    let path = pending_enrollment_path(session_id)?;
    let raw = fs::read(&path).map_err(|error| {
        format!(
            "failed to read pending enrollment from {}: {error}",
            path.display()
        )
    })?;
    serde_json::from_slice(&raw).map_err(|error| {
        format!(
            "failed to decode pending enrollment from {}: {error}",
            path.display()
        )
    })
}

fn delete_pending_enrollment(session_id: &str) -> Result<(), String> {
    let path = pending_enrollment_path(session_id)?;
    if !path.exists() {
        return Ok(());
    }

    fs::remove_file(&path).map_err(|error| {
        format!(
            "failed to delete pending enrollment {}: {error}",
            path.display()
        )
    })
}

fn verify_hs256_token(token: &str) -> Result<TokenPayload, AuthnError> {
    let segments = token.split('.').map(str::trim).collect::<Vec<_>>();
    if segments.len() != 3 || segments.iter().any(|segment| segment.is_empty()) {
        return Err(AuthnError::InvalidCredential);
    }

    let header: JwtHeader = decode_json_segment(segments[0])?;
    if header.alg != "HS256" {
        return Err(AuthnError::InvalidCredential);
    }

    let payload: TokenPayload = decode_json_segment(segments[1])?;
    let provided_signature = URL_SAFE_NO_PAD
        .decode(segments[2])
        .map_err(|_| AuthnError::InvalidCredential)?;
    let signing_input = format!("{}.{}", segments[0], segments[1]);
    let mut mac = HmacSha256::new_from_slice(jwt_secret().as_bytes()).map_err(|error| {
        AuthnError::InternalError(format!("failed to initialize HMAC verifier: {error}"))
    })?;
    mac.update(signing_input.as_bytes());
    mac.verify_slice(&provided_signature)
        .map_err(|_| AuthnError::InvalidCredential)?;

    if let Some(exp) = payload.exp {
        let now = unix_timestamp_seconds().map_err(|error| {
            AuthnError::InternalError(format!("failed to read system clock: {error}"))
        })?;
        if now >= exp {
            return Err(AuthnError::Expired);
        }
    }

    Ok(payload)
}

fn decode_json_segment<T>(segment: &str) -> Result<T, AuthnError>
where
    T: for<'de> Deserialize<'de>,
{
    let decoded = URL_SAFE_NO_PAD
        .decode(segment)
        .map_err(|_| AuthnError::InvalidCredential)?;
    serde_json::from_slice(&decoded).map_err(|error| {
        AuthnError::InternalError(format!("failed to decode JWT payload: {error}"))
    })
}

fn issue_jwt(
    subject: &str,
    roles: &[String],
    scopes: &[String],
    ttl: Duration,
) -> Result<String, String> {
    let header = serde_json::json!({
        "alg": "HS256",
        "typ": "JWT",
    });
    let payload = serde_json::json!({
        "sub": subject,
        "roles": normalize_roles(roles.to_vec()),
        "scopes": normalize_scopes(scopes.to_vec()),
        "exp": unix_timestamp_seconds().map_err(|error| error.to_string())? + ttl.as_secs(),
    });
    let encoded_header = URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&header)
            .map_err(|error| format!("failed to encode JWT header: {error}"))?,
    );
    let encoded_payload = URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&payload)
            .map_err(|error| format!("failed to encode JWT payload: {error}"))?,
    );
    let signing_input = format!("{encoded_header}.{encoded_payload}");
    let mut mac = HmacSha256::new_from_slice(jwt_secret().as_bytes())
        .map_err(|error| format!("failed to initialize JWT signer: {error}"))?;
    mac.update(signing_input.as_bytes());
    let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    Ok(format!("{signing_input}.{signature}"))
}

fn jwt_secret() -> String {
    env::var(JWT_SECRET_ENV).unwrap_or_else(|_| DEFAULT_JWT_SECRET.to_owned())
}

fn unix_timestamp_seconds() -> Result<u64, std::time::SystemTimeError> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}

fn normalize_username(username: &str) -> Result<String, String> {
    let trimmed = username.trim();
    if trimmed.is_empty() {
        return Err("username must not be empty".to_owned());
    }

    Ok(trimmed.to_ascii_lowercase())
}

fn normalize_person_name(value: &str, label: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    if trimmed.len() > 64 {
        return Err(format!("{label} must be 64 characters or fewer"));
    }
    Ok(trimmed.to_owned())
}

fn normalize_password(password: &str) -> Result<&str, String> {
    let trimmed = password.trim();
    if trimmed.len() < 8 {
        return Err("password must be at least 8 characters long".to_owned());
    }
    Ok(trimmed)
}

fn normalize_session_id(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("session id must not be empty".to_owned());
    }
    sanitize_filename(trimmed)
}

fn normalize_pat_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("PAT name must not be empty".to_owned());
    }
    if trimmed.len() > 64 {
        return Err("PAT name must be 64 characters or fewer".to_owned());
    }
    Ok(trimmed.to_owned())
}

fn normalize_roles(roles: Vec<String>) -> Vec<String> {
    let mut roles = roles
        .into_iter()
        .map(|role| role.trim().to_ascii_lowercase())
        .filter(|role| !role.is_empty())
        .collect::<Vec<_>>();
    roles.sort();
    roles.dedup();
    roles
}

fn normalize_scopes(scopes: Vec<String>) -> Vec<String> {
    let mut scopes = scopes
        .into_iter()
        .flat_map(|scope| {
            scope
                .split(',')
                .map(str::trim)
                .filter(|scope| !scope.is_empty())
                .map(str::to_ascii_lowercase)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    scopes.sort();
    scopes.dedup();
    scopes
}

fn hash_user_password(username: &str, password: &str) -> Result<String, String> {
    let password = normalize_password(password)?;
    let mut hasher = Sha256::new();
    hasher.update(username.as_bytes());
    hasher.update(b":");
    hasher.update(password.as_bytes());
    Ok(hex::encode(hasher.finalize()))
}

fn generate_pat_token() -> String {
    let suffix = Alphanumeric
        .sample_string(&mut rand::rng(), PAT_TOKEN_CHARS)
        .to_ascii_lowercase();
    format!("{PAT_PREFIX}{suffix}")
}

fn generate_pending_enrollment_id() -> String {
    Alphanumeric
        .sample_string(&mut rand::rng(), 32)
        .to_ascii_lowercase()
}

fn generate_totp_secret() -> String {
    let mut secret = [0_u8; TOTP_SECRET_BYTES];
    for byte in &mut secret {
        *byte = rand::random::<u8>();
    }
    base32_encode(&secret)
}

fn build_totp_provisioning_uri(username: &str, secret: &str) -> String {
    let issuer = percent_encode(TOTP_ISSUER);
    let label = percent_encode(&format!("{TOTP_ISSUER}:{username}"));
    format!(
        "otpauth://totp/{label}?secret={secret}&issuer={issuer}&algorithm=SHA1&digits={TOTP_DIGITS}&period={TOTP_PERIOD_SECONDS}"
    )
}

fn verify_totp_code(secret: &str, code: &str, now: u64) -> Result<(), String> {
    let normalized = code.trim().replace(' ', "");
    if normalized.len() != TOTP_DIGITS as usize
        || !normalized
            .chars()
            .all(|character| character.is_ascii_digit())
    {
        return Err("TOTP code must be a 6-digit number".to_owned());
    }

    let secret = base32_decode(secret)?;
    let counter = now / TOTP_PERIOD_SECONDS;

    for skew in -TOTP_ALLOWED_SKEW_STEPS..=TOTP_ALLOWED_SKEW_STEPS {
        let candidate = if skew < 0 {
            counter.checked_sub(skew.unsigned_abs())
        } else {
            counter.checked_add(skew as u64)
        };
        let Some(candidate) = candidate else {
            continue;
        };

        if format!(
            "{:0width$}",
            totp_value(&secret, candidate),
            width = TOTP_DIGITS as usize
        ) == normalized
        {
            return Ok(());
        }
    }

    Err("invalid TOTP code".to_owned())
}

fn totp_value(secret: &[u8], counter: u64) -> u32 {
    let digest = hmac_sha1(secret, &counter.to_be_bytes());
    let offset = (digest[digest.len() - 1] & 0x0f) as usize;
    let truncated = ((digest[offset] as u32 & 0x7f) << 24)
        | ((digest[offset + 1] as u32) << 16)
        | ((digest[offset + 2] as u32) << 8)
        | (digest[offset + 3] as u32);
    truncated % 10_u32.pow(TOTP_DIGITS)
}

fn hmac_sha1(key: &[u8], message: &[u8]) -> [u8; 20] {
    const BLOCK_SIZE: usize = 64;

    let mut normalized_key = if key.len() > BLOCK_SIZE {
        let mut hasher = Sha1::new();
        hasher.update(key);
        hasher.finalize().to_vec()
    } else {
        key.to_vec()
    };
    normalized_key.resize(BLOCK_SIZE, 0);

    let mut inner_pad = [0x36_u8; BLOCK_SIZE];
    let mut outer_pad = [0x5c_u8; BLOCK_SIZE];
    for (index, byte) in normalized_key.iter().enumerate() {
        inner_pad[index] ^= *byte;
        outer_pad[index] ^= *byte;
    }

    let mut inner = Sha1::new();
    inner.update(inner_pad);
    inner.update(message);
    let inner_digest = inner.finalize();

    let mut outer = Sha1::new();
    outer.update(outer_pad);
    outer.update(inner_digest);
    let digest = outer.finalize();

    let mut output = [0_u8; 20];
    output.copy_from_slice(&digest);
    output
}

fn base32_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

    let mut output = String::new();
    let mut buffer = 0_u16;
    let mut bits = 0_u8;

    for byte in bytes {
        buffer = (buffer << 8) | u16::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let index = ((buffer >> bits) & 0x1f) as usize;
            output.push(ALPHABET[index] as char);
        }
    }

    if bits > 0 {
        let index = ((buffer << (5 - bits)) & 0x1f) as usize;
        output.push(ALPHABET[index] as char);
    }

    output
}

fn base32_decode(value: &str) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    let mut buffer = 0_u32;
    let mut bits = 0_u8;

    for character in value.trim().chars() {
        if character == '=' {
            break;
        }
        let upper = character.to_ascii_uppercase();
        let digit = match upper {
            'A'..='Z' => upper as u32 - 'A' as u32,
            '2'..='7' => upper as u32 - '2' as u32 + 26,
            _ => return Err("TOTP secret is not valid base32".to_owned()),
        };

        buffer = (buffer << 5) | digit;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            output.push(((buffer >> bits) & 0xff) as u8);
        }
    }

    if output.is_empty() {
        return Err("TOTP secret must not be empty".to_owned());
    }

    Ok(output)
}

fn percent_encode(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                vec![byte as char]
            }
            _ => format!("%{byte:02X}").chars().collect::<Vec<_>>(),
        })
        .collect()
}

fn hash_registration_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"registration:");
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

fn generate_recovery_code() -> String {
    let chunk = || {
        Alphanumeric
            .sample_string(&mut rand::rng(), 5)
            .to_ascii_uppercase()
    };

    format!("TCHN-{}-{}", chunk(), chunk())
}

fn normalize_recovery_code(code: &str) -> Result<String, String> {
    let normalized = code.trim().to_ascii_uppercase();
    let parts = normalized.split('-').collect::<Vec<_>>();
    if parts.len() != 3
        || parts[0] != "TCHN"
        || parts[1].len() != 5
        || parts[2].len() != 5
        || !parts[1]
            .chars()
            .chain(parts[2].chars())
            .all(|character| character.is_ascii_uppercase() || character.is_ascii_digit())
    {
        return Err("recovery code must match TCHN-XXXXX-XXXXX".to_owned());
    }

    Ok(normalized)
}

fn hash_recovery_code(username: &str, code: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(username.as_bytes());
    hasher.update(b":");
    hasher.update(code.as_bytes());
    hex::encode(hasher.finalize())
}

fn hash_pat_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

fn load_user_record(username: &str) -> Result<UserSecurityRecord, String> {
    let path = user_record_path(username)?;
    if !path.exists() {
        return Ok(UserSecurityRecord::default());
    }

    let raw = fs::read(&path)
        .map_err(|error| format!("failed to read auth state from {}: {error}", path.display()))?;
    serde_json::from_slice(&raw).map_err(|error| {
        format!(
            "failed to decode auth state from {}: {error}",
            path.display()
        )
    })
}

fn save_user_record(username: &str, record: &UserSecurityRecord) -> Result<(), String> {
    let path = user_record_path(username)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create auth state directory {}: {error}",
                parent.display()
            )
        })?;
    }

    let payload = serde_json::to_vec_pretty(record)
        .map_err(|error| format!("failed to encode auth state: {error}"))?;
    fs::write(&path, payload).map_err(|error| {
        format!(
            "failed to persist auth state to {}: {error}",
            path.display()
        )
    })
}

fn user_record_path(username: &str) -> Result<PathBuf, String> {
    let sanitized = sanitize_filename(username)?;
    Ok(state_root_dir().join(format!("{sanitized}.json")))
}

fn pending_enrollment_dir() -> PathBuf {
    state_root_dir().join("pending-enrollments")
}

fn pending_enrollment_path(session_id: &str) -> Result<PathBuf, String> {
    let sanitized = sanitize_filename(session_id)?;
    Ok(pending_enrollment_dir().join(format!("{sanitized}.json")))
}

fn registration_token_store_path() -> PathBuf {
    state_root_dir()
        .join("registration-tokens")
        .join("consumed.json")
}

fn load_consumed_registration_tokens() -> Result<ConsumedRegistrationTokenStore, String> {
    let path = registration_token_store_path();
    if !path.exists() {
        return Ok(ConsumedRegistrationTokenStore::default());
    }

    let raw = fs::read(&path).map_err(|error| {
        format!(
            "failed to read consumed registration token store from {}: {error}",
            path.display()
        )
    })?;
    serde_json::from_slice(&raw).map_err(|error| {
        format!(
            "failed to decode consumed registration token store from {}: {error}",
            path.display()
        )
    })
}

fn save_consumed_registration_tokens(store: &ConsumedRegistrationTokenStore) -> Result<(), String> {
    let path = registration_token_store_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create registration token store directory {}: {error}",
                parent.display()
            )
        })?;
    }

    let payload = serde_json::to_vec_pretty(store)
        .map_err(|error| format!("failed to encode consumed registration token store: {error}"))?;
    fs::write(&path, payload).map_err(|error| {
        format!(
            "failed to persist consumed registration token store to {}: {error}",
            path.display()
        )
    })
}

fn state_root_dir() -> PathBuf {
    env::var_os(AUTH_STATE_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn sanitize_filename(value: &str) -> Result<String, String> {
    let sanitized = value
        .chars()
        .map(|character| match character {
            'a'..='z' | '0'..='9' | '-' | '_' => character,
            _ => '_',
        })
        .collect::<String>();
    if sanitized.is_empty() || sanitized == "." || sanitized == ".." {
        return Err("username resolves to an invalid auth state key".to_owned());
    }
    Ok(sanitized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::RngExt as _;
    use serde_json::json;
    use std::sync::Mutex;

    static TEST_ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_test_env<T>(test: impl FnOnce() -> T) -> T {
        let _guard = TEST_ENV_LOCK
            .lock()
            .expect("test environment lock should not be poisoned");
        test()
    }

    fn with_temp_state_dir<T>(test: impl FnOnce() -> T) -> T {
        let temp_dir = std::env::temp_dir().join(format!(
            "tachyon-authn-test-{}",
            rand::rng().random::<u64>()
        ));
        fs::create_dir_all(&temp_dir).expect("temporary auth state directory should exist");
        std::env::set_var(AUTH_STATE_DIR_ENV, &temp_dir);
        let result = test();
        std::env::remove_var(AUTH_STATE_DIR_ENV);
        let _ = fs::remove_dir_all(temp_dir);
        result
    }

    fn issue_registration_token_for_test(
        secret: &str,
        subject: &str,
        roles: &[&str],
        scopes: &[&str],
        issued_at: u64,
        expires_at: u64,
    ) -> String {
        std::env::set_var(JWT_SECRET_ENV, secret);
        let header = json!({
            "alg": "HS256",
            "typ": "JWT",
        });
        let payload = json!({
            "sub": subject,
            "iat": issued_at,
            "exp": expires_at,
            "token_use": REGISTRATION_TOKEN_USE,
            "invite_roles": roles,
            "invite_scopes": scopes,
        });
        let encoded_header = URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&header).expect("header should encode for test token"));
        let encoded_payload = URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&payload).expect("payload should encode for test token"));
        let signing_input = format!("{encoded_header}.{encoded_payload}");
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
            .expect("test HMAC signer should initialize");
        mac.update(signing_input.as_bytes());
        let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        format!("{signing_input}.{signature}")
    }

    fn totp_code_for_test(secret: &str, timestamp: u64) -> String {
        let secret = base32_decode(secret).expect("test secret should decode");
        format!(
            "{:0width$}",
            totp_value(&secret, timestamp / TOTP_PERIOD_SECONDS),
            width = TOTP_DIGITS as usize
        )
    }

    fn provisioning_secret_for_test(uri: &str) -> String {
        uri.split("secret=")
            .nth(1)
            .and_then(|segment| segment.split('&').next())
            .expect("provisioning URI should expose a secret")
            .to_owned()
    }

    #[test]
    fn generated_recovery_codes_match_expected_format() {
        with_test_env(|| {
            let code = generate_recovery_code();

            assert!(code.starts_with("TCHN-"));
            assert_eq!(code.len(), "TCHN-XXXXX-XXXXX".len());
            assert!(normalize_recovery_code(&code).is_ok());
        });
    }

    #[test]
    fn issued_tokens_round_trip_through_verifier() {
        with_test_env(|| {
            std::env::set_var(JWT_SECRET_ENV, "unit-test-secret");
            let token = issue_jwt(
                "admin@example.test",
                &[String::from("admin"), String::from("ops")],
                &[String::from("manage:tokens")],
                Duration::from_secs(60),
            )
            .expect("token should be issued");

            let claims = validate_identity_token(&token).expect("token should validate");

            assert_eq!(claims.subject, "admin@example.test");
            assert_eq!(claims.roles, vec!["admin".to_owned(), "ops".to_owned()]);
            assert_eq!(claims.scopes, vec!["manage:tokens".to_owned()]);
            std::env::remove_var(JWT_SECRET_ENV);
        });
    }

    #[test]
    fn malformed_recovery_codes_are_rejected() {
        with_test_env(|| {
            let error =
                normalize_recovery_code("not-a-code").expect_err("invalid code should fail");

            assert_eq!(error, "recovery code must match TCHN-XXXXX-XXXXX");
        });
    }

    #[test]
    fn issued_pat_round_trips_through_validator() {
        with_test_env(|| {
            with_temp_state_dir(|| {
                let token = issue_pat_token(
                    "admin",
                    "GitHub Actions",
                    &[String::from("deploy:wasm"), String::from("read:nodes")],
                    7,
                )
                .expect("PAT should be issued");

                let identity = validate_identity_token(&token).expect("PAT should validate");
                assert_eq!(identity.subject, "admin");
                assert!(identity.roles.is_empty());
                assert_eq!(
                    identity.scopes,
                    vec!["deploy:wasm".to_owned(), "read:nodes".to_owned()]
                );
            });
        });
    }

    #[test]
    fn registration_tokens_stage_and_finalize_once() {
        with_test_env(|| {
            with_temp_state_dir(|| {
                let secret = "signup-unit-test-secret";
                let issued_at = unix_timestamp_seconds().expect("clock should be available");
                let expires_at = issued_at + 300;
                let token = issue_registration_token_for_test(
                    secret,
                    "invite:ops-admin",
                    &["admin", "ops"],
                    &["deploy:wasm", "read:nodes"],
                    issued_at,
                    expires_at,
                );

                let claims = validate_registration_token_claims(&token)
                    .expect("registration token should pass");
                assert_eq!(claims.subject, "invite:ops-admin");
                assert_eq!(claims.roles, vec!["admin".to_owned(), "ops".to_owned()]);

                let staged = stage_pending_user(
                    &token,
                    SignupProfile {
                        first_name: "Jane".to_owned(),
                        last_name: "Mesh".to_owned(),
                        username: "jane".to_owned(),
                        password: "correct horse battery staple".to_owned(),
                    },
                )
                .expect("signup session should stage");

                let code = totp_code_for_test(
                    &provisioning_secret_for_test(&staged.provisioning_uri),
                    issued_at,
                );
                let session = finalize_pending_enrollment(&staged.session_id, &code)
                    .expect("signup enrollment should finalize");

                assert_eq!(session.username, "jane");
                assert_eq!(session.roles, vec!["admin".to_owned(), "ops".to_owned()]);
                assert!(validate_registration_token_claims(&token).is_err());
            });
            std::env::remove_var(JWT_SECRET_ENV);
        });
    }
}

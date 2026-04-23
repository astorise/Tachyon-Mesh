mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/authn.wit",
        world: "authn-guest",
    });

    export!(Component);
}

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use bindings::exports::tachyon::identity::authn::{AuthnError, IdentityPayload};
use hmac::{Hmac, KeyInit, Mac};
use rand::distr::{Alphanumeric, SampleString};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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
}

impl bindings::exports::tachyon::identity::authn::Guest for Component {
    fn validate_token(token: String) -> Result<IdentityPayload, AuthnError> {
        validate_identity_token(&token)
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

fn generate_pat_token() -> String {
    let suffix = Alphanumeric
        .sample_string(&mut rand::rng(), PAT_TOKEN_CHARS)
        .to_ascii_lowercase();
    format!("{PAT_PREFIX}{suffix}")
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

    #[test]
    fn generated_recovery_codes_match_expected_format() {
        let code = generate_recovery_code();

        assert!(code.starts_with("TCHN-"));
        assert_eq!(code.len(), "TCHN-XXXXX-XXXXX".len());
        assert!(normalize_recovery_code(&code).is_ok());
    }

    #[test]
    fn issued_tokens_round_trip_through_verifier() {
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
    }

    #[test]
    fn malformed_recovery_codes_are_rejected() {
        let error = normalize_recovery_code("not-a-code").expect_err("invalid code should fail");

        assert_eq!(error, "recovery code must match TCHN-XXXXX-XXXXX");
    }

    #[test]
    fn issued_pat_round_trips_through_validator() {
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
    }
}

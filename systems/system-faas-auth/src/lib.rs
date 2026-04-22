mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/identity.wit",
        world: "auth-guest",
    });

    export!(Component);
}

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use bindings::exports::tachyon::identity::auth::{AuthError, Claims};
use hmac::{Hmac, KeyInit, Mac};
use rand::{distributions::Alphanumeric, Rng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const DEFAULT_JWT_SECRET: &str = "tachyon-dev-secret";
const JWT_SECRET_ENV: &str = "TACHYON_AUTH_JWT_SECRET";
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
    exp: Option<u64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct UserSecurityRecord {
    #[serde(default)]
    recovery_hashes: Vec<String>,
}

impl bindings::exports::tachyon::identity::auth::Guest for Component {
    fn verify_token(token: String, required_roles: Vec<String>) -> Result<Claims, AuthError> {
        let payload = verify_hs256_token(&token)?;

        if !required_roles.is_empty()
            && !payload
                .roles
                .iter()
                .any(|role| required_roles.iter().any(|required| required == role))
        {
            return Err(AuthError::MissingRoles);
        }

        Ok(Claims {
            subject: payload.subject,
            roles: payload.roles,
        })
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
            Duration::from_secs(RECOVERY_SESSION_TTL_SECONDS),
        )
    }
}

fn verify_hs256_token(token: &str) -> Result<TokenPayload, AuthError> {
    let segments = token.split('.').map(str::trim).collect::<Vec<_>>();
    if segments.len() != 3 || segments.iter().any(|segment| segment.is_empty()) {
        return Err(AuthError::InvalidSignature);
    }

    let header: JwtHeader = decode_json_segment(segments[0])?;
    if header.alg != "HS256" {
        return Err(AuthError::InvalidSignature);
    }

    let payload: TokenPayload = decode_json_segment(segments[1])?;
    let provided_signature = URL_SAFE_NO_PAD
        .decode(segments[2])
        .map_err(|_| AuthError::InvalidSignature)?;
    let signing_input = format!("{}.{}", segments[0], segments[1]);
    let mut mac = HmacSha256::new_from_slice(jwt_secret().as_bytes()).map_err(|error| {
        AuthError::InternalError(format!("failed to initialize HMAC verifier: {error}"))
    })?;
    mac.update(signing_input.as_bytes());
    mac.verify_slice(&provided_signature)
        .map_err(|_| AuthError::InvalidSignature)?;

    if let Some(exp) = payload.exp {
        let now = unix_timestamp_seconds().map_err(|error| {
            AuthError::InternalError(format!("failed to read system clock: {error}"))
        })?;
        if now >= exp {
            return Err(AuthError::Expired);
        }
    }

    Ok(payload)
}

fn decode_json_segment<T>(segment: &str) -> Result<T, AuthError>
where
    T: for<'de> Deserialize<'de>,
{
    let decoded = URL_SAFE_NO_PAD
        .decode(segment)
        .map_err(|_| AuthError::InvalidSignature)?;
    serde_json::from_slice(&decoded)
        .map_err(|error| AuthError::InternalError(format!("failed to decode JWT payload: {error}")))
}

fn issue_jwt(subject: &str, roles: &[String], ttl: Duration) -> Result<String, String> {
    let header = serde_json::json!({
        "alg": "HS256",
        "typ": "JWT",
    });
    let payload = serde_json::json!({
        "sub": subject,
        "roles": roles,
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

fn generate_recovery_code() -> String {
    let chunk = || {
        rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(5)
            .map(char::from)
            .map(|character| character.to_ascii_uppercase())
            .collect::<String>()
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
    Ok(state_root_dir()?.join(format!("{sanitized}.json")))
}

fn state_root_dir() -> Result<PathBuf, String> {
    let path = PathBuf::from(".");
    fs::create_dir_all(&path).map_err(|error| {
        format!(
            "failed to initialize auth state directory {}: {error}",
            path.display()
        )
    })?;
    Ok(path)
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
            Duration::from_secs(60),
        )
        .expect("token should be issued");

        let claims = verify_hs256_token(&token).expect("token should validate");

        assert_eq!(claims.subject, "admin@example.test");
        assert_eq!(claims.roles, vec!["admin".to_owned(), "ops".to_owned()]);
        std::env::remove_var(JWT_SECRET_ENV);
    }

    #[test]
    fn malformed_recovery_codes_are_rejected() {
        let error = normalize_recovery_code("not-a-code").expect_err("invalid code should fail");

        assert_eq!(error, "recovery code must match TCHN-XXXXX-XXXXX");
    }
}

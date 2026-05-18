use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

use uuid::Uuid;

pub const SECRET_PLACEHOLDER_PREFIX: &str = "tachyon:secret:";
const CANONICAL_UUID_LENGTH: usize = 36;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretRegistry {
    pub id: Uuid,
    pub plaintext: String,
    pub allowed_hosts: Vec<String>,
}

static SECRET_REGISTRY: OnceLock<Mutex<HashMap<Uuid, SecretRegistry>>> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<Uuid, SecretRegistry>> {
    SECRET_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(test)]
pub fn register_secret(
    id: Uuid,
    plaintext: impl Into<String>,
    allowed_hosts: Vec<String>,
) -> Result<(), String> {
    let mut guard = registry()
        .lock()
        .map_err(|_| "secret registry lock is poisoned".to_owned())?;
    guard.insert(
        id,
        SecretRegistry {
            id,
            plaintext: plaintext.into(),
            allowed_hosts,
        },
    );
    Ok(())
}

#[cfg(test)]
pub fn clear_secrets() -> Result<(), String> {
    registry()
        .lock()
        .map_err(|_| "secret registry lock is poisoned".to_owned())?
        .clear();
    Ok(())
}

#[cfg(test)]
pub fn test_registry_guard() -> std::sync::MutexGuard<'static, ()> {
    static TEST_REGISTRY_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    TEST_REGISTRY_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("secret registry test lock should not be poisoned")
}

pub fn resolve_secret(placeholder: &str, target_host: &str) -> Result<String, String> {
    let id = parse_placeholder_id(placeholder)?;
    let target_host = normalize_host(target_host)
        .ok_or_else(|| "target host is required to resolve a secret placeholder".to_owned())?;
    let guard = registry()
        .lock()
        .map_err(|_| "secret registry lock is poisoned".to_owned())?;
    let secret = guard
        .get(&id)
        .ok_or_else(|| format!("secret placeholder `{id}` is not registered"))?;

    if secret_host_allowed(&secret.allowed_hosts, &target_host) {
        Ok(secret.plaintext.clone())
    } else {
        Err(format!(
            "secret placeholder `{id}` is not allowed for target host `{target_host}`"
        ))
    }
}

pub fn parse_placeholder_id(placeholder: &str) -> Result<Uuid, String> {
    let trimmed = placeholder.trim();
    let id = trimmed
        .strip_prefix(SECRET_PLACEHOLDER_PREFIX)
        .ok_or_else(|| {
            format!("secret placeholder must start with `{SECRET_PLACEHOLDER_PREFIX}`")
        })?;
    Uuid::parse_str(id).map_err(|error| format!("invalid secret placeholder UUID `{id}`: {error}"))
}

pub fn placeholder_token_at(candidate: &str) -> Option<&str> {
    if !candidate.starts_with(SECRET_PLACEHOLDER_PREFIX) {
        return None;
    }
    let token_len = SECRET_PLACEHOLDER_PREFIX
        .len()
        .saturating_add(CANONICAL_UUID_LENGTH);
    let token = candidate.get(..token_len)?;
    parse_placeholder_id(token).ok()?;
    Some(token)
}

fn secret_host_allowed(allowed_hosts: &[String], target_host: &str) -> bool {
    allowed_hosts.iter().any(|allowed| {
        let Some(allowed) = normalize_host(allowed) else {
            return false;
        };
        allowed == "*"
            || allowed.eq_ignore_ascii_case(target_host)
            || allowed
                .strip_prefix("*.")
                .is_some_and(|suffix| target_host.ends_with(suffix) && target_host != suffix)
    })
}

fn normalize_host(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_end_matches('.');
    if trimmed.is_empty() {
        return None;
    }
    if trimmed == "*" {
        return Some(trimmed.to_owned());
    }
    if let Ok(url) = reqwest::Url::parse(trimmed) {
        return url
            .host_str()
            .map(|host| host.trim_end_matches('.').to_ascii_lowercase());
    }

    let host = trimmed.split('/').next().unwrap_or(trimmed);
    let host = host
        .strip_prefix('[')
        .and_then(|value| value.split(']').next())
        .unwrap_or_else(|| host.split(':').next().unwrap_or(host));
    let host = host.trim().trim_end_matches('.');
    if host.is_empty() {
        None
    } else {
        Some(host.to_ascii_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_secret_returns_plaintext_for_allowed_host() {
        let _guard = test_registry_guard();
        clear_secrets().expect("registry should clear");
        let id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000")
            .expect("static UUID should parse");
        register_secret(id, "sk-live", vec!["https://api.openai.com/v1".to_owned()])
            .expect("secret should register");

        let resolved = resolve_secret(
            "tachyon:secret:550e8400-e29b-41d4-a716-446655440000",
            "api.openai.com",
        )
        .expect("secret should resolve");

        assert_eq!(resolved, "sk-live");
    }

    #[test]
    fn resolve_secret_rejects_disallowed_host() {
        let _guard = test_registry_guard();
        clear_secrets().expect("registry should clear");
        let id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440001")
            .expect("static UUID should parse");
        register_secret(id, "sk-live", vec!["api.openai.com".to_owned()])
            .expect("secret should register");

        let error = resolve_secret(
            "tachyon:secret:550e8400-e29b-41d4-a716-446655440001",
            "evil.test",
        )
        .expect_err("wrong host should be rejected");

        assert!(error.contains("not allowed"));
    }
}

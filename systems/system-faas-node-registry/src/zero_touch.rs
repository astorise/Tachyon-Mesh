//! Pure (I/O-free) machine-identity evaluation for zero-touch enrollment.
//!
//! The FaaS `lib.rs` performs the I/O (JWKS fetch, signer call) and delegates
//! the trust decisions here so they are unit-testable. Signature verification
//! itself lives in [`crate::jwt`].

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde_json::Value;

/// Decode the JWT header and return its `kid` (key id), if present.
pub fn header_kid(token: &str) -> Result<Option<String>, String> {
    let part = token.split('.').next().unwrap_or_default();
    let header = URL_SAFE_NO_PAD
        .decode(part)
        .map_err(|e| format!("invalid JWT header: {e}"))?;
    let header: Value =
        serde_json::from_slice(&header).map_err(|e| format!("invalid JWT header json: {e}"))?;
    Ok(header
        .get("kid")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned))
}

/// Parse the JWT payload bytes into a claims object.
pub fn claims_from_payload(payload: &[u8]) -> Result<Value, String> {
    serde_json::from_slice(payload).map_err(|e| format!("invalid JWT claims json: {e}"))
}

/// Select the `(n, e)` modulus/exponent of the JWKS key matching `kid`
/// (or the sole key when no `kid` is present).
pub fn select_jwk<'a>(jwks: &'a Value, kid: Option<&str>) -> Result<(&'a str, &'a str), String> {
    let keys = jwks
        .get("keys")
        .and_then(Value::as_array)
        .ok_or_else(|| "JWKS has no `keys` array".to_owned())?;
    let key = keys
        .iter()
        .find(|k| match kid {
            Some(kid) => k.get("kid").and_then(Value::as_str) == Some(kid),
            None => true,
        })
        .ok_or_else(|| "no JWKS key matched the token `kid`".to_owned())?;
    let n = key
        .get("n")
        .and_then(Value::as_str)
        .ok_or_else(|| "JWKS key missing `n`".to_owned())?;
    let e = key
        .get("e")
        .and_then(Value::as_str)
        .ok_or_else(|| "JWKS key missing `e`".to_owned())?;
    Ok((n, e))
}

/// Reject expired tokens (with a small leeway) and, when an audience is
/// configured, tokens whose `aud` does not contain it.
pub fn check_aud_exp(claims: &Value, audience: Option<&str>, now: u64) -> Result<(), String> {
    const LEEWAY_SECONDS: u64 = 60;
    let exp = claims
        .get("exp")
        .and_then(Value::as_u64)
        .ok_or_else(|| "token has no `exp`".to_owned())?;
    if now > exp + LEEWAY_SECONDS {
        return Err("token is expired".to_owned());
    }
    if let Some(audience) = audience {
        let ok = match claims.get("aud") {
            Some(Value::String(s)) => s == audience,
            Some(Value::Array(items)) => items.iter().any(|v| v.as_str() == Some(audience)),
            _ => false,
        };
        if !ok {
            return Err(format!("token `aud` does not contain `{audience}`"));
        }
    }
    Ok(())
}

/// The token subject (`sub`), or an empty string.
pub fn subject(claims: &Value) -> String {
    claims
        .get("sub")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// Match every `key=value` matcher in `tags` against the validated claims.
/// Returns the matched matchers on success, or an error naming the first miss.
/// Empty `tags` never auto-approves (no policy = no automatic trust).
pub fn match_tags(claims: &Value, tags: &[String]) -> Result<Vec<String>, String> {
    if tags.is_empty() {
        return Err("no auto_approve_tags configured; refusing to auto-approve".to_owned());
    }
    for tag in tags {
        let (key, want) = tag
            .split_once('=')
            .ok_or_else(|| format!("auto_approve_tags entry `{tag}` is not key=value"))?;
        let got = resolve_claim(claims, key.trim());
        if got.as_deref() != Some(want.trim()) {
            return Err(format!(
                "claim `{}` did not match `{}`",
                key.trim(),
                want.trim()
            ));
        }
    }
    Ok(tags.to_vec())
}

/// Resolve a tag key to a claim value, flattening the well-known Kubernetes
/// ServiceAccount claims so policies can match on `namespace`/`serviceaccount`.
fn resolve_claim(claims: &Value, key: &str) -> Option<String> {
    let k8s = claims.get("kubernetes.io");
    match key {
        "namespace" => k8s
            .and_then(|v| v.get("namespace"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        "serviceaccount" => k8s
            .and_then(|v| v.get("serviceaccount"))
            .and_then(|v| v.get("name"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        "pod" => k8s
            .and_then(|v| v.get("pod"))
            .and_then(|v| v.get("name"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        other => claims
            .get(other)
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn k8s_claims() -> Value {
        json!({
            "sub": "system:serviceaccount:tachyon:tachyon-node",
            "aud": ["tachyon"],
            "exp": 10_000,
            "kubernetes.io": {
                "namespace": "tachyon",
                "serviceaccount": { "name": "tachyon-node" }
            }
        })
    }

    #[test]
    fn matches_k8s_namespace_and_serviceaccount() {
        let claims = k8s_claims();
        let tags = vec![
            "namespace=tachyon".to_owned(),
            "serviceaccount=tachyon-node".to_owned(),
        ];
        let matched = match_tags(&claims, &tags).expect("claims should match");
        assert_eq!(matched, tags);
    }

    #[test]
    fn rejects_non_matching_namespace() {
        let claims = k8s_claims();
        let tags = vec!["namespace=default".to_owned()];
        assert!(match_tags(&claims, &tags).is_err());
    }

    #[test]
    fn empty_tags_never_auto_approves() {
        assert!(match_tags(&k8s_claims(), &[]).is_err());
    }

    #[test]
    fn audience_and_expiry_are_enforced() {
        let claims = k8s_claims();
        assert!(check_aud_exp(&claims, Some("tachyon"), 9_000).is_ok());
        assert!(check_aud_exp(&claims, Some("other"), 9_000).is_err());
        assert!(check_aud_exp(&claims, Some("tachyon"), 20_000).is_err());
    }

    #[test]
    fn selects_jwk_by_kid() {
        let jwks = json!({ "keys": [
            { "kid": "a", "n": "AA", "e": "AQAB" },
            { "kid": "b", "n": "BB", "e": "AQAB" }
        ]});
        let (n, _e) = select_jwk(&jwks, Some("b")).expect("kid b present");
        assert_eq!(n, "BB");
        assert!(select_jwk(&jwks, Some("z")).is_err());
    }
}

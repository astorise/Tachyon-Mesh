#![allow(dead_code)]

pub(crate) mod authn {
    pub(crate) const MODULE: &str = "identity::authn";
}

pub(crate) mod authz {
    pub(crate) const MODULE: &str = "identity::authz";
}

pub(crate) mod enrollment {
    pub(crate) const MODULE: &str = "identity::enrollment";
}

use super::*;

// Extracted caller identity signing and verification.

// Extracted caller identity signing and verification.

// Extracted caller identity signing and verification.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) struct CallerIdentityClaims {
    pub(crate) route_path: String,
    pub(crate) role: RouteRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) tenant_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) token_id: Option<String>,
    pub(crate) issued_at: u64,
    pub(crate) expires_at: u64,
}

#[derive(Clone)]
pub(crate) struct HostIdentity {
    pub(crate) signing_key: Arc<SigningKey>,
    pub(crate) public_key: VerifyingKey,
    pub(crate) public_key_hex: String,
}

impl HostIdentity {
    pub(crate) fn generate() -> Self {
        Self::from_signing_key(SigningKey::from_bytes(&rand::random::<[u8; 32]>()))
    }

    pub(crate) fn from_signing_key(signing_key: SigningKey) -> Self {
        let public_key = signing_key.verifying_key();
        Self {
            signing_key: Arc::new(signing_key),
            public_key_hex: hex::encode(public_key.to_bytes()),
            public_key,
        }
    }

    pub(crate) fn sign_route(&self, route: &IntegrityRoute) -> Result<String> {
        let now = unix_timestamp_seconds()?;
        self.sign_claims(&CallerIdentityClaims {
            route_path: normalize_route_path(&route.path),
            role: route.role,
            tenant_id: None,
            token_id: None,
            issued_at: now,
            expires_at: now.saturating_add(IDENTITY_TOKEN_TTL.as_secs()),
        })
    }

    pub(crate) fn sign_claims(&self, claims: &CallerIdentityClaims) -> Result<String> {
        let payload =
            serde_json::to_vec(claims).context("failed to serialize signed caller identity")?;
        let signature = self.signing_key.sign(&payload);
        Ok(format!(
            "{IDENTITY_TOKEN_PREFIX}.{}.{}",
            hex::encode(payload),
            hex::encode(signature.to_bytes())
        ))
    }

    pub(crate) fn verify_header(
        &self,
        headers: &HeaderMap,
    ) -> std::result::Result<CallerIdentityClaims, String> {
        let raw = headers
            .get(TACHYON_IDENTITY_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| forbidden_error("missing host-signed caller identity"))?;
        let token = raw
            .strip_prefix("Bearer ")
            .or_else(|| raw.strip_prefix("bearer "))
            .unwrap_or(raw);
        self.verify_token(token)
            .map_err(|error| forbidden_error(&error))
    }

    pub(crate) fn verify_token(
        &self,
        token: &str,
    ) -> std::result::Result<CallerIdentityClaims, String> {
        let Some(rest) = token.strip_prefix(&format!("{IDENTITY_TOKEN_PREFIX}.")) else {
            return Err("caller identity token has an invalid prefix".to_owned());
        };
        let Some((payload_hex, signature_hex)) = rest.split_once('.') else {
            return Err("caller identity token is malformed".to_owned());
        };
        let payload = hex::decode(payload_hex)
            .map_err(|_| "caller identity token payload is not valid hex".to_owned())?;
        let signature_bytes = decode_hex_array::<64>(signature_hex, "caller identity signature")
            .map_err(|error| error.to_string())?;
        let signature = Signature::from_bytes(&signature_bytes);

        self.public_key
            .verify(&payload, &signature)
            .map_err(|_| "caller identity token signature verification failed".to_owned())?;

        let claims: CallerIdentityClaims = serde_json::from_slice(&payload)
            .map_err(|_| "caller identity token payload is not valid JSON".to_owned())?;
        let now = unix_timestamp_seconds().map_err(|error| error.to_string())?;
        if claims.issued_at > claims.expires_at {
            return Err("caller identity token timestamps are invalid".to_owned());
        }
        if now > claims.expires_at {
            return Err("caller identity token has expired".to_owned());
        }

        Ok(claims)
    }
}

//! Pure-Rust (wasm-safe) RS256 verification for machine-identity enrollment
//! tokens. No `ring`/`jsonwebtoken` (not wasm-friendly).

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rsa::pkcs1v15::{Signature, VerifyingKey};
use rsa::signature::Verifier;
use rsa::{BigUint, RsaPublicKey};
use sha2::Sha256;

/// Verify an RS256 JWT signature given the modulus `n` and exponent `e`
/// (base64url, as published in a JWKS). `signing_input` is `header.payload`.
pub fn verify_rs256(
    signing_input: &str,
    signature_b64url: &str,
    n_b64url: &str,
    e_b64url: &str,
) -> Result<(), String> {
    let n = URL_SAFE_NO_PAD
        .decode(n_b64url)
        .map_err(|e| format!("invalid JWK modulus: {e}"))?;
    let e = URL_SAFE_NO_PAD
        .decode(e_b64url)
        .map_err(|e| format!("invalid JWK exponent: {e}"))?;
    let sig_bytes = URL_SAFE_NO_PAD
        .decode(signature_b64url)
        .map_err(|e| format!("invalid JWT signature: {e}"))?;

    let public_key = RsaPublicKey::new(BigUint::from_bytes_be(&n), BigUint::from_bytes_be(&e))
        .map_err(|e| format!("invalid RSA public key: {e}"))?;
    let verifying_key = VerifyingKey::<Sha256>::new(public_key);
    let signature = Signature::try_from(sig_bytes.as_slice())
        .map_err(|e| format!("malformed signature: {e}"))?;
    verifying_key
        .verify(signing_input.as_bytes(), &signature)
        .map_err(|_| "JWT signature verification failed".to_owned())
}

/// Split a compact JWT into (`signing_input`, `signature_b64url`, decoded payload bytes).
pub fn split_jwt(token: &str) -> Result<(String, String, Vec<u8>), String> {
    let parts: Vec<&str> = token.trim().split('.').collect();
    if parts.len() != 3 {
        return Err("token is not a compact JWT".to_owned());
    }
    let signing_input = format!("{}.{}", parts[0], parts[1]);
    let payload = URL_SAFE_NO_PAD
        .decode(parts[1])
        .map_err(|e| format!("invalid JWT payload: {e}"))?;
    Ok((signing_input, parts[2].to_owned(), payload))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::pkcs1v15::SigningKey as RsaSigningKey;
    use rsa::signature::{SignatureEncoding, Signer};
    use rsa::traits::PublicKeyParts;
    use rsa::RsaPrivateKey;

    #[test]
    fn split_rejects_non_compact_tokens() {
        assert!(split_jwt("only.two").is_err());
        // header.{"sub":"x"}.sig — middle segment must be valid base64url.
        let (signing_input, sig, payload) =
            split_jwt("aGVhZGVy.eyJzdWIiOiJ4In0.c2ln").expect("compact JWT splits");
        assert_eq!(signing_input, "aGVhZGVy.eyJzdWIiOiJ4In0");
        assert_eq!(sig, "c2ln");
        assert_eq!(&payload, br#"{"sub":"x"}"#);
    }

    #[test]
    fn rs256_round_trip_verifies_and_rejects_tampering() {
        // Generate a real RSA key, sign a JWT signing-input, then verify with the
        // public (n, e) exactly as a JWKS would publish them.
        let mut rng = rand::thread_rng();
        let private = RsaPrivateKey::new(&mut rng, 2048).expect("rsa keygen");
        let signing_key = RsaSigningKey::<Sha256>::new(private.clone());

        let signing_input = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJzeXN0ZW0ifQ";
        let signature = signing_key.sign(signing_input.as_bytes());
        let signature_b64 = URL_SAFE_NO_PAD.encode(signature.to_bytes());
        let n_b64 = URL_SAFE_NO_PAD.encode(private.n().to_bytes_be());
        let e_b64 = URL_SAFE_NO_PAD.encode(private.e().to_bytes_be());

        // Correct signature verifies.
        verify_rs256(signing_input, &signature_b64, &n_b64, &e_b64)
            .expect("valid RS256 signature must verify");

        // Tampered signing input is rejected.
        assert!(verify_rs256("tampered.payload", &signature_b64, &n_b64, &e_b64).is_err());

        // A different key (wrong modulus) is rejected.
        let other = RsaPrivateKey::new(&mut rng, 2048).expect("rsa keygen");
        let other_n = URL_SAFE_NO_PAD.encode(other.n().to_bytes_be());
        assert!(verify_rs256(signing_input, &signature_b64, &other_n, &e_b64).is_err());
    }
}

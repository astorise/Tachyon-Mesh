mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "system-faas-guest",
    });

    export!(Component);
}

use bindings::tachyon::mesh::storage_broker::WriteMode;
use ed25519_dalek::{Signer, SigningKey};
use serde::{Deserialize, Serialize};

/// Wire value for the issuance mode the host sends (`mode=ACME_STAGING_MOCK`).
/// The label is historical: issuance now mints a real cluster-CA-signed leaf,
/// but the host/guest contract for this string is fixed and must not change.
const CA_ISSUED_MODE: &str = "ACME_STAGING_MOCK";
/// Internal route the node-registry FaaS calls to mint a node credential.
const SIGN_NODE_ROUTE: &str = "/internal/cert-manager/sign-node";
/// Mounted path of the 32-byte cluster-CA ed25519 seed (raw or hex). Only
/// signer-eligible pods mount this secret.
const CA_SEED_PATH: &str = "/ca/cluster-ca.seed";
/// Route-env carrying the cluster-CA seed (hex), passed into the guest sandbox.
const CA_SEED_ENV: &str = "TACHYON_CLUSTER_CA_SEED";

struct Component;

#[derive(Debug, PartialEq, Eq)]
struct CertificateRequest {
    domain: String,
    storage_path: String,
    mode: String,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct CertificateBundle {
    domain: String,
    certificate_pem: String,
    private_key_pem: String,
}

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        if !req.method.eq_ignore_ascii_case("POST") {
            return response(405, "Method Not Allowed");
        }

        // Cluster-CA node-credential signing for zero-touch enrollment. The
        // node-registry FaaS calls this route after validating a machine
        // identity; we mint an ed25519-signed credential over the node key.
        let path = req.uri.split_once('?').map(|(p, _)| p).unwrap_or(&req.uri);
        if path == SIGN_NODE_ROUTE {
            return match sign_node_credential(&req.body) {
                Ok(credential_hex) => response(200, credential_hex),
                Err((status, message)) => response(status, message),
            };
        }

        let request = match parse_certificate_request(&req.uri) {
            Ok(request) => request,
            Err(message) => return response(400, message),
        };

        if request.mode != CA_ISSUED_MODE {
            return response(400, "cert-manager `mode` must be `ACME_STAGING_MOCK`");
        }

        let bundle = match issue_leaf_certificate(&request.domain) {
            Ok(bundle) => bundle,
            Err(message) => return response(500, message),
        };

        let body = match serde_json::to_vec(&bundle) {
            Ok(body) => body,
            Err(error) => {
                return response(
                    500,
                    format!("failed to serialize certificate bundle: {error}"),
                );
            }
        };

        if let Err(error) = bindings::tachyon::mesh::storage_broker::enqueue_write(
            &request.storage_path,
            WriteMode::Overwrite,
            &body,
        ) {
            let (status, body) = map_storage_error(error);
            return response(status, body);
        }

        response(200, body)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignNodeRequest {
    node_public_key: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NodeCredential {
    node_public_key: String,
    ca_public_key: String,
    issued_at: u64,
    signature: String,
}

/// Mint a cluster-CA-signed credential for a node's ed25519 public key.
/// Returns the hex-encoded credential bytes (the node-registry FaaS stages
/// this hex for the enrolling node's poll, which stores the decoded bytes).
fn sign_node_credential(body: &[u8]) -> Result<Vec<u8>, (u16, String)> {
    let request: SignNodeRequest = serde_json::from_slice(body)
        .map_err(|error| (400, format!("invalid sign-node request: {error}")))?;
    let signing_key = load_ca_signing_key()?;
    sign_node_credential_with_key(&signing_key, request.node_public_key.trim())
}

/// Pure credential minting (no I/O), separated so the signing scheme is
/// unit-testable: ed25519 signature over the node's public-key bytes, wrapped in
/// a JSON `NodeCredential` and hex-encoded.
fn sign_node_credential_with_key(
    signing_key: &SigningKey,
    node_public_key: &str,
) -> Result<Vec<u8>, (u16, String)> {
    let node_public_key = node_public_key.trim().to_owned();
    let node_key_bytes = hex::decode(&node_public_key)
        .map_err(|error| (400, format!("nodePublicKey must be hex: {error}")))?;
    let signature = signing_key.sign(&node_key_bytes);
    let credential = NodeCredential {
        node_public_key,
        ca_public_key: hex::encode(signing_key.verifying_key().to_bytes()),
        issued_at: now_seconds(),
        signature: hex::encode(signature.to_bytes()),
    };
    let json = serde_json::to_vec(&credential)
        .map_err(|error| (500, format!("failed to encode node credential: {error}")))?;
    Ok(hex::encode(json).into_bytes())
}

fn load_ca_signing_key() -> Result<SigningKey, (u16, String)> {
    // Prefer the env-provided seed (set via the sealed route `env`, which the
    // host passes into the guest sandbox). A WASM guest cannot read an
    // arbitrary host mount like `/ca` unless it is a configured volume, so env
    // is the portable source; fall back to the mounted file when present.
    if let Ok(env_seed) = std::env::var(CA_SEED_ENV) {
        let seed = parse_ca_seed(env_seed.as_bytes()).ok_or_else(|| {
            (
                500,
                format!("{CA_SEED_ENV} must be 32 raw bytes or 64 hex chars"),
            )
        })?;
        return Ok(SigningKey::from_bytes(&seed));
    }
    let raw = std::fs::read(CA_SEED_PATH).map_err(|error| {
        (
            503,
            format!(
                "cluster CA seed unavailable: set {CA_SEED_ENV} or mount {CA_SEED_PATH} ({error})"
            ),
        )
    })?;
    let seed = parse_ca_seed(&raw).ok_or_else(|| {
        (
            500,
            format!("cluster CA seed at {CA_SEED_PATH} must be 32 raw bytes or 64 hex chars"),
        )
    })?;
    Ok(SigningKey::from_bytes(&seed))
}

/// Accept the CA seed as 32 raw bytes or a 64-char hex string (with optional
/// surrounding whitespace).
fn parse_ca_seed(raw: &[u8]) -> Option<[u8; 32]> {
    if raw.len() == 32 {
        let mut seed = [0u8; 32];
        seed.copy_from_slice(raw);
        return Some(seed);
    }
    let text = std::str::from_utf8(raw).ok()?.trim();
    let decoded = hex::decode(text).ok()?;
    if decoded.len() == 32 {
        let mut seed = [0u8; 32];
        seed.copy_from_slice(&decoded);
        Some(seed)
    } else {
        None
    }
}

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

fn parse_certificate_request(uri: &str) -> Result<CertificateRequest, &'static str> {
    let query = uri
        .split_once('?')
        .map(|(_, query)| query)
        .ok_or("cert-manager requests must include query parameters")?;
    let mut domain = None;
    let mut storage_path = None;
    let mut mode = None;

    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "domain" => domain = Some(value.to_owned()),
            "storage_path" => storage_path = Some(value.to_owned()),
            "mode" => mode = Some(value.to_owned()),
            _ => {}
        }
    }

    let domain = require_query_value(domain, "domain")?;
    if domain.contains('/') || domain.contains('\\') || domain.contains(' ') {
        return Err("cert-manager `domain` must be a single hostname");
    }

    Ok(CertificateRequest {
        domain,
        storage_path: require_query_value(storage_path, "storage_path")?,
        mode: require_query_value(mode, "mode")?,
    })
}

fn require_query_value(value: Option<String>, key: &'static str) -> Result<String, &'static str> {
    let value = value.ok_or(match key {
        "domain" => "cert-manager requests must include `domain`",
        "storage_path" => "cert-manager requests must include `storage_path`",
        "mode" => "cert-manager requests must include `mode`",
        _ => "cert-manager request is missing a required query parameter",
    })?;

    if value.trim().is_empty() {
        Err(match key {
            "domain" => "cert-manager `domain` must not be empty",
            "storage_path" => "cert-manager `storage_path` must not be empty",
            "mode" => "cert-manager `mode` must not be empty",
            _ => "cert-manager query parameter must not be empty",
        })
    } else {
        Ok(value)
    }
}

/// Issue a real X.509 leaf certificate for `domain`, signed by the cluster CA.
///
/// This is the single (wasm-safe) issuance path: it loads the cluster-CA
/// signing key, derives the deterministic CA certificate, generates a fresh
/// random ECDSA P-256 leaf keypair, and mints a v3 leaf certificate
/// (`CN=<domain>`, SAN dNSName=<domain>, serverAuth) signed over the
/// TBSCertificate DER with the CA key. The returned bundle is a leaf-first
/// full chain plus the leaf's PKCS#8 private key.
///
/// The cluster CA remains ed25519, so the certificate's `signatureAlgorithm`
/// stays Ed25519 (1.3.101.112); only the leaf's subjectPublicKeyInfo and the
/// returned leaf private key are P-256. A P-256 leaf is universally accepted
/// for TLS server CertificateVerify, whereas an ed25519 leaf breaks clients
/// that do not advertise the ed25519 signature scheme.
fn issue_leaf_certificate(domain: &str) -> Result<CertificateBundle, String> {
    let ca_signing_key = load_ca_signing_key().map_err(|(_, message)| message)?;
    cert::issue_leaf_certificate(&ca_signing_key, domain)
}

/// Pure X.509 issuance (no host I/O), separated so the certificate scheme is
/// unit-testable on the host target. Mints a cluster-CA-signed leaf for
/// `domain` using `ca_signing_key` as the issuing CA.
mod cert {
    use super::{now_seconds, CertificateBundle, SigningKey};
    use const_oid::db::rfc5280::ID_KP_SERVER_AUTH;
    use const_oid::db::rfc8410::ID_ED_25519;
    use const_oid::AssociatedOid;
    use der::asn1::{BitString, GeneralizedTime, Ia5String, OctetString, UtcTime};
    use der::flagset::FlagSet;
    use der::{Decode, Encode, EncodePem};
    use ed25519_dalek::Signer;
    use pkcs8::EncodePrivateKey;
    use spki::{AlgorithmIdentifierOwned, EncodePublicKey, SubjectPublicKeyInfoOwned};
    use std::str::FromStr;
    use std::time::Duration;
    use x509_cert::certificate::{Certificate, TbsCertificate, Version};
    use x509_cert::ext::pkix::name::GeneralName;
    use x509_cert::ext::pkix::{
        BasicConstraints, ExtendedKeyUsage, KeyUsage, KeyUsages, SubjectAltName,
    };
    use x509_cert::ext::{Extension, Extensions};
    use x509_cert::name::Name;
    use x509_cert::serial_number::SerialNumber;
    use x509_cert::time::{Time, Validity};

    /// Subject/issuer common name of the self-signed cluster CA.
    const CA_COMMON_NAME: &str = "CN=tachyon-cluster-ca";
    /// Fixed CA `notBefore` (2020-01-01T00:00:00Z) so the CA certificate is
    /// deterministic for a given CA key.
    const CA_NOT_BEFORE_SECS: u64 = 1_577_836_800;
    /// CA certificate lifetime past `now` (~10 years).
    const CA_VALIDITY_SECS: u64 = 10 * 365 * 24 * 60 * 60;
    /// Leaf certificate lifetime past `now` (~90 days).
    const LEAF_VALIDITY_SECS: u64 = 90 * 24 * 60 * 60;

    pub(super) fn issue_leaf_certificate(
        ca_signing_key: &SigningKey,
        domain: &str,
    ) -> Result<CertificateBundle, String> {
        let now = now_seconds();

        // Deterministic self-signed CA certificate for the issuing key.
        let ca_cert = build_ca_certificate(ca_signing_key, now)?;
        let ca_pem = ca_cert
            .to_pem(der::pem::LineEnding::LF)
            .map_err(|error| format!("failed to PEM-encode CA certificate: {error}"))?;

        // Fresh random ECDSA P-256 leaf keypair.
        let leaf_signing_key = generate_leaf_key();
        let leaf_cert = build_leaf_certificate(ca_signing_key, &leaf_signing_key, domain, now)?;
        let leaf_pem = leaf_cert
            .to_pem(der::pem::LineEnding::LF)
            .map_err(|error| format!("failed to PEM-encode leaf certificate: {error}"))?;
        let leaf_key_pem = leaf_signing_key
            .to_pkcs8_pem(der::pem::LineEnding::LF)
            .map_err(|error| format!("failed to PEM-encode leaf private key: {error}"))?
            .to_string();

        // Full chain, leaf-first: the host loads every cert in the PEM as the
        // served chain.
        Ok(CertificateBundle {
            domain: domain.to_owned(),
            certificate_pem: format!("{leaf_pem}{ca_pem}"),
            private_key_pem: leaf_key_pem,
        })
    }

    /// Generate a fresh random ECDSA P-256 (NIST secp256r1) leaf keypair.
    ///
    /// `OsRng` is getrandom-backed (WASI on wasm32-wasip2), the same entropy
    /// source the cluster CA seed loading relies on.
    fn generate_leaf_key() -> p256::ecdsa::SigningKey {
        p256::ecdsa::SigningKey::random(&mut rand_core::OsRng)
    }

    /// Build the deterministic self-signed cluster-CA certificate.
    fn build_ca_certificate(ca_signing_key: &SigningKey, now: u64) -> Result<Certificate, String> {
        let name = distinguished_name(CA_COMMON_NAME)?;
        let validity = validity(CA_NOT_BEFORE_SECS, now.saturating_add(CA_VALIDITY_SECS))?;
        let extensions = vec![
            basic_constraints_extension(true)?,
            key_usage_extension(KeyUsages::KeyCertSign | KeyUsages::DigitalSignature)?,
        ];
        // The CA is self-signed: its SPKI is the ed25519 CA verifying key.
        let spki = subject_public_key_info(&ca_signing_key.verifying_key())?;
        sign_certificate(
            ca_signing_key,
            spki,
            serial_number(now, 0)?,
            name.clone(),
            name,
            validity,
            extensions,
        )
    }

    /// Build the cluster-CA-signed v3 leaf certificate for `domain`.
    ///
    /// The leaf key is ECDSA P-256, so the leaf's subjectPublicKeyInfo carries
    /// `id-ecPublicKey` with the P-256 named curve. The certificate is still
    /// signed by the ed25519 cluster CA (see `sign_certificate`).
    fn build_leaf_certificate(
        ca_signing_key: &SigningKey,
        leaf_signing_key: &p256::ecdsa::SigningKey,
        domain: &str,
        now: u64,
    ) -> Result<Certificate, String> {
        let subject = distinguished_name(&format!("CN={domain}"))?;
        let issuer = distinguished_name(CA_COMMON_NAME)?;
        let validity = validity(now, now.saturating_add(LEAF_VALIDITY_SECS))?;
        let extensions = vec![
            basic_constraints_extension(false)?,
            key_usage_extension(KeyUsages::DigitalSignature.into())?,
            extended_key_usage_extension()?,
            subject_alt_name_extension(domain)?,
        ];
        // The leaf's SPKI is the P-256 verifying key.
        let spki = subject_public_key_info(leaf_signing_key.verifying_key())?;
        sign_certificate(
            ca_signing_key,
            spki,
            serial_number(now, 1)?,
            issuer,
            subject,
            validity,
            extensions,
        )
    }

    /// Assemble a `TbsCertificate`, sign its DER with the CA key (Ed25519), and
    /// wrap it into a `Certificate`.
    ///
    /// `subject_public_key_info` is supplied by the caller so the same signing
    /// path serves both the ed25519 self-signed CA (CA SPKI) and the P-256 leaf
    /// (leaf SPKI). The signature itself is always the ed25519 CA signature over
    /// the TBSCertificate DER, so the certificate's `signatureAlgorithm` stays
    /// Ed25519 (1.3.101.112) regardless of the subject key type.
    fn sign_certificate(
        ca_signing_key: &SigningKey,
        subject_public_key_info: SubjectPublicKeyInfoOwned,
        serial_number: SerialNumber,
        issuer: Name,
        subject: Name,
        validity: Validity,
        extensions: Extensions,
    ) -> Result<Certificate, String> {
        let signature_algorithm = ed25519_algorithm_identifier();
        let tbs_certificate = TbsCertificate {
            version: Version::V3,
            serial_number,
            signature: signature_algorithm.clone(),
            issuer,
            validity,
            subject,
            subject_public_key_info,
            issuer_unique_id: None,
            subject_unique_id: None,
            extensions: Some(extensions),
        };

        let tbs_der = tbs_certificate
            .to_der()
            .map_err(|error| format!("failed to encode TBSCertificate: {error}"))?;
        let signature = ca_signing_key.sign(&tbs_der);
        let signature_bits = BitString::from_bytes(&signature.to_bytes())
            .map_err(|error| format!("failed to encode certificate signature: {error}"))?;

        Ok(Certificate {
            tbs_certificate,
            signature_algorithm,
            signature: signature_bits,
        })
    }

    /// `AlgorithmIdentifier` for Ed25519 (OID 1.3.101.112, absent parameters).
    fn ed25519_algorithm_identifier() -> AlgorithmIdentifierOwned {
        AlgorithmIdentifierOwned {
            oid: ID_ED_25519,
            parameters: None,
        }
    }

    /// Build the SPKI for any verifying key that can encode itself as a
    /// SubjectPublicKeyInfo DER. Used for both the ed25519 CA key (yielding an
    /// Ed25519 SPKI) and the P-256 leaf key (yielding an `id-ecPublicKey` SPKI
    /// with the P-256 named curve).
    fn subject_public_key_info<T: EncodePublicKey>(
        verifying_key: &T,
    ) -> Result<SubjectPublicKeyInfoOwned, String> {
        let der = verifying_key
            .to_public_key_der()
            .map_err(|error| format!("failed to encode subject public key: {error}"))?;
        SubjectPublicKeyInfoOwned::from_der(der.as_bytes())
            .map_err(|error| format!("failed to parse subject public key info: {error}"))
    }

    fn distinguished_name(rfc4514: &str) -> Result<Name, String> {
        Name::from_str(rfc4514)
            .map_err(|error| format!("failed to build distinguished name: {error}"))
    }

    /// Positive serial number derived from the issuance time plus a per-cert
    /// discriminant so CA and leaf never collide.
    fn serial_number(now: u64, discriminant: u8) -> Result<SerialNumber, String> {
        let mut bytes = [0u8; 9];
        bytes[..8].copy_from_slice(&now.to_be_bytes());
        bytes[8] = discriminant;
        SerialNumber::new(&bytes).map_err(|error| format!("failed to build serial number: {error}"))
    }

    fn validity(not_before_secs: u64, not_after_secs: u64) -> Result<Validity, String> {
        Ok(Validity {
            not_before: x509_time(not_before_secs)?,
            not_after: x509_time(not_after_secs)?,
        })
    }

    /// Encode a unix timestamp as an X.509 `Time`, using `UTCTime` through 2049
    /// (per RFC 5280 4.1.2.5) and `GeneralizedTime` thereafter.
    fn x509_time(secs: u64) -> Result<Time, String> {
        let duration = Duration::from_secs(secs);
        // 2050-01-01T00:00:00Z: RFC 5280 requires GeneralizedTime from 2050 on.
        const UTC_TIME_MAX_SECS: u64 = 2_524_608_000;
        if secs < UTC_TIME_MAX_SECS {
            let utc = UtcTime::from_unix_duration(duration)
                .map_err(|error| format!("failed to build UTCTime: {error}"))?;
            Ok(Time::UtcTime(utc))
        } else {
            let general = GeneralizedTime::from_unix_duration(duration)
                .map_err(|error| format!("failed to build GeneralizedTime: {error}"))?;
            Ok(Time::GeneralTime(general))
        }
    }

    fn basic_constraints_extension(is_ca: bool) -> Result<Extension, String> {
        let basic_constraints = BasicConstraints {
            ca: is_ca,
            path_len_constraint: None,
        };
        build_extension(true, &basic_constraints)
    }

    fn key_usage_extension(usages: FlagSet<KeyUsages>) -> Result<Extension, String> {
        build_extension(true, &KeyUsage(usages))
    }

    fn extended_key_usage_extension() -> Result<Extension, String> {
        build_extension(false, &ExtendedKeyUsage(vec![ID_KP_SERVER_AUTH]))
    }

    fn subject_alt_name_extension(domain: &str) -> Result<Extension, String> {
        let dns = Ia5String::new(domain)
            .map_err(|error| format!("domain is not a valid dNSName: {error}"))?;
        build_extension(false, &SubjectAltName(vec![GeneralName::DnsName(dns)]))
    }

    /// Wrap an encodable extension value into an `Extension`, taking its OID from
    /// the value's `AssociatedOid`.
    fn build_extension<T: Encode + AssociatedOid>(
        critical: bool,
        value: &T,
    ) -> Result<Extension, String> {
        let der = value
            .to_der()
            .map_err(|error| format!("failed to encode extension value: {error}"))?;
        let extn_value = OctetString::new(der)
            .map_err(|error| format!("failed to wrap extension value: {error}"))?;
        Ok(Extension {
            extn_id: T::OID,
            critical,
            extn_value,
        })
    }
}

fn map_storage_error(error: String) -> (u16, Vec<u8>) {
    if let Some(message) = error.strip_prefix("forbidden:") {
        return (403, message.trim().as_bytes().to_vec());
    }

    (500, error.into_bytes())
}

fn response(
    status: u16,
    body: impl Into<Vec<u8>>,
) -> bindings::exports::tachyon::mesh::handler::Response {
    bindings::exports::tachyon::mesh::handler::Response {
        status,
        headers: vec![],
        body: body.into(),
        trailers: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_certificate_request_extracts_expected_fields() {
        let request = parse_certificate_request(
            "/system/cert-manager?domain=api.example.test&storage_path=/app/certs/api.example.test.json&mode=ACME_STAGING_MOCK",
        )
        .expect("request should parse");

        assert_eq!(
            request,
            CertificateRequest {
                domain: "api.example.test".to_owned(),
                storage_path: "/app/certs/api.example.test.json".to_owned(),
                mode: CA_ISSUED_MODE.to_owned(),
            }
        );
        assert_eq!(CA_ISSUED_MODE, "ACME_STAGING_MOCK");
    }

    #[test]
    fn parse_certificate_request_rejects_invalid_domain() {
        let error = parse_certificate_request(
            "/system/cert-manager?domain=api/example.test&storage_path=/app/certs/api.example.test.json&mode=ACME_STAGING_MOCK",
        )
        .expect_err("invalid domain should fail");

        assert_eq!(error, "cert-manager `domain` must be a single hostname");
    }

    // ---- X.509 leaf issuance validation -----------------------------------
    //
    // `TACHYON_CLUSTER_CA_SEED` is process-global, so tests that mutate it are
    // serialized behind this lock to avoid cross-test interference.
    static CA_SEED_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// The fixed CA seed used by the issuance tests (64 hex chars / 32 bytes).
    const TEST_CA_SEED_HEX: &str =
        "0101010101010101010101010101010101010101010101010101010101010101";

    fn test_ca_signing_key() -> SigningKey {
        let seed: [u8; 32] = hex::decode(TEST_CA_SEED_HEX)
            .expect("seed is hex")
            .try_into()
            .expect("32-byte seed");
        SigningKey::from_bytes(&seed)
    }

    fn issue_via_env(domain: &str) -> CertificateBundle {
        let _guard = CA_SEED_ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        std::env::set_var(CA_SEED_ENV, TEST_CA_SEED_HEX);
        let bundle = issue_leaf_certificate(domain).expect("leaf certificate should be issued");
        std::env::remove_var(CA_SEED_ENV);
        bundle
    }

    /// Test 1: issuance succeeds when the CA seed is provided via env, exactly
    /// how `load_ca_signing_key` reads it in production.
    #[test]
    fn issues_leaf_for_domain_from_env_seed() {
        let bundle = issue_via_env("api.example.test");
        assert_eq!(bundle.domain, "api.example.test");
        // Full chain, leaf-first: exactly two certificates in the PEM.
        assert_eq!(
            bundle.certificate_pem.matches("BEGIN CERTIFICATE").count(),
            2,
            "certificate_pem must contain the leaf followed by the CA cert"
        );
        assert!(bundle.private_key_pem.contains("BEGIN PRIVATE KEY"));
    }

    /// Test 2: the leaf parses and carries the expected subject, SAN and issuer.
    #[test]
    fn leaf_certificate_has_expected_names_and_san() {
        use x509_parser::prelude::*;

        let domain = "node-7.api.example.test";
        let bundle = cert::issue_leaf_certificate(&test_ca_signing_key(), domain).expect("issued");
        let leaf_der = first_certificate_der(&bundle.certificate_pem);
        let (_, leaf) = X509Certificate::from_der(&leaf_der).expect("leaf parses");

        let subject_cn = leaf
            .subject()
            .iter_common_name()
            .next()
            .and_then(|cn| cn.as_str().ok())
            .expect("subject CN present");
        assert_eq!(subject_cn, domain);

        let issuer_cn = leaf
            .issuer()
            .iter_common_name()
            .next()
            .and_then(|cn| cn.as_str().ok())
            .expect("issuer CN present");
        assert_eq!(issuer_cn, "tachyon-cluster-ca");

        let san = leaf
            .subject_alternative_name()
            .expect("SAN extension parses")
            .expect("SAN extension present");
        let has_dns = san
            .value
            .general_names
            .iter()
            .any(|name| matches!(name, GeneralName::DNSName(dns) if *dns == domain));
        assert!(has_dns, "SAN must contain dNSName={domain}");
    }

    /// Test 3: the leaf certificate's Ed25519 signature validates under the CA
    /// verifying key (TBSCertificate bytes verified against the CA public key),
    /// proving it is genuinely CA-signed.
    #[test]
    fn leaf_certificate_is_signed_by_the_cluster_ca() {
        use ed25519_dalek::{Signature, Verifier};
        use x509_parser::prelude::*;

        let ca_signing_key = test_ca_signing_key();
        let bundle =
            cert::issue_leaf_certificate(&ca_signing_key, "api.example.test").expect("issued");
        let leaf_der = first_certificate_der(&bundle.certificate_pem);
        let (_, leaf) = X509Certificate::from_der(&leaf_der).expect("leaf parses");

        // Ed25519 signature OID (1.3.101.112).
        assert_eq!(
            leaf.signature_algorithm.algorithm.to_id_string(),
            "1.3.101.112"
        );

        let tbs = leaf.tbs_certificate.as_ref();
        let sig_bytes: [u8; 64] = leaf
            .signature_value
            .as_ref()
            .try_into()
            .expect("ed25519 signature is 64 bytes");
        let signature = Signature::from_bytes(&sig_bytes);

        ca_signing_key
            .verifying_key()
            .verify(tbs, &signature)
            .expect("leaf TBS must verify under the cluster CA key");

        // A different CA key must reject the same leaf.
        let other_ca = SigningKey::from_bytes(&[42u8; 32]).verifying_key();
        assert!(
            other_ca.verify(tbs, &signature).is_err(),
            "leaf must not verify under an unrelated CA key"
        );
    }

    /// Test 4: rustls accepts the full chain + leaf key via `with_single_cert`,
    /// proving the ed25519 leaf and PKCS#8 key are mutually consistent and
    /// loadable by the host TLS runtime.
    #[test]
    fn bundle_loads_into_a_rustls_server_config() {
        use std::io::BufReader;

        let _ = rustls::crypto::ring::default_provider().install_default();

        let bundle = cert::issue_leaf_certificate(&test_ca_signing_key(), "api.example.test")
            .expect("issued");

        let mut cert_reader = BufReader::new(bundle.certificate_pem.as_bytes());
        let cert_chain = rustls_pemfile::certs(&mut cert_reader)
            .collect::<Result<Vec<_>, _>>()
            .expect("certificate chain parses");
        assert_eq!(cert_chain.len(), 2, "chain must be leaf + CA");

        let mut key_reader = BufReader::new(bundle.private_key_pem.as_bytes());
        let private_key = rustls_pemfile::private_key(&mut key_reader)
            .expect("private key parses")
            .expect("private key present");

        rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(cert_chain, private_key)
            .expect("rustls must accept the ed25519 leaf and key");
    }

    /// Test 5: each issuance mints a fresh random leaf key (never the old mock).
    #[test]
    fn each_issuance_uses_a_fresh_leaf_key() {
        let ca_signing_key = test_ca_signing_key();
        let first =
            cert::issue_leaf_certificate(&ca_signing_key, "api.example.test").expect("issued");
        let second =
            cert::issue_leaf_certificate(&ca_signing_key, "api.example.test").expect("issued");

        assert_ne!(
            first.private_key_pem, second.private_key_pem,
            "leaf private key must be randomly generated per issuance"
        );
        assert_ne!(
            first.certificate_pem, second.certificate_pem,
            "fresh leaf key must yield a different certificate"
        );

        // It must not be the retired hardcoded mock material.
        let old_mock_cn = "api.example.test";
        assert!(
            !first
                .private_key_pem
                .contains("MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcw"),
            "issuance must not return the old hardcoded RSA mock key"
        );
        // The new leaf key is PKCS#8 ed25519 (short), not the long RSA mock.
        assert!(
            first.private_key_pem.len() < 400,
            "ed25519 PKCS#8 keys are short"
        );
        // Subject is still the requested domain (sanity, unrelated to mock).
        let _ = old_mock_cn;
    }

    /// Helper: decode the first PEM certificate (the leaf) into DER.
    fn first_certificate_der(pem: &str) -> Vec<u8> {
        use std::io::BufReader;
        let mut reader = BufReader::new(pem.as_bytes());
        let mut certs = rustls_pemfile::certs(&mut reader);
        let first = certs
            .next()
            .expect("at least one certificate")
            .expect("certificate parses");
        first.as_ref().to_vec()
    }

    #[test]
    fn node_credential_is_signed_and_verifiable_by_the_cluster_ca() {
        // Zero-touch validation (unknown #2, soundness): the minted credential
        // is an ed25519 signature over the node public-key bytes, verifiable by
        // the cluster-CA public key and rejected by any other key.
        use ed25519_dalek::{Signature, Verifier};

        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let ca_verifying = signing_key.verifying_key();
        let node_pub = "11223344556677889900aabbccddeeff";

        let hex_credential = sign_node_credential_with_key(&signing_key, node_pub)
            .expect("credential should be minted");
        let json = hex::decode(&hex_credential).expect("credential body is hex");
        let credential: serde_json::Value =
            serde_json::from_slice(&json).expect("credential is json");

        assert_eq!(credential["nodePublicKey"], node_pub);
        assert_eq!(
            credential["caPublicKey"]
                .as_str()
                .expect("caPublicKey is a string"),
            hex::encode(ca_verifying.to_bytes())
        );

        let message = hex::decode(node_pub).expect("node pubkey is hex");
        let sig_hex = credential["signature"]
            .as_str()
            .expect("signature is a string");
        let sig_bytes: [u8; 64] = hex::decode(sig_hex)
            .expect("signature is hex")
            .try_into()
            .expect("64-byte signature");
        ca_verifying
            .verify(&message, &Signature::from_bytes(&sig_bytes))
            .expect("cluster CA must verify its own credential");

        let other_ca = SigningKey::from_bytes(&[9u8; 32]).verifying_key();
        assert!(
            other_ca
                .verify(&message, &Signature::from_bytes(&sig_bytes))
                .is_err(),
            "a credential must not verify under a different CA key"
        );
    }
}

# Proposal: FIPS Compliant TLS Runtime

## Context
Tachyon Mesh relies heavily on `rustls` for mTLS, HTTPS, and Quic. By default, `rustls` uses the `ring` cryptographic provider. While highly secure, `ring` is not globally FIPS 140-2/3 certified. To sell or deploy Tachyon into US Government (FedRAMP), European banking, or defense networks, we must provide a mathematically equivalent but legally certified cryptographic backend.

## Proposed Solution
We will leverage `rustls`'s pluggable crypto providers and the `aws-lc-rs` crate (which wraps AWS libcrypto, a FIPS-validated module).
1. Add a Cargo feature flag: `--features fips`.
2. When compiled with this flag, `core-host` will initialize `rustls` using the `aws-lc-rs` FIPS provider instead of the default `ring` provider.
3. If the host boots in FIPS mode, it will aggressively reject any non-FIPS compliant cipher suites (e.g., forcing AES-GCM and strict TLS 1.3).
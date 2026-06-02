## 1. Enrollment config block

- [x] 1.1 Add an optional `enrollment` block to `IntegrityConfig` in `core-host/src/host_core/domain_types.rs`: `mode` (`pin` | `zero-touch` | `both`, default `pin`), `oidc_issuer: Option<String>`, `oidc_audience: Option<String>`, `auto_approve_tags: Vec<String>` (all `skip_serializing_if`-empty for back-compat)
- [x] 1.2 Add validation in `integrity_config.rs`: reject `zero-touch`/`both` without a non-empty `oidc_issuer`; reject `auto_approve_tags` entries not of the form `key=value`
- [x] 1.3 Unit tests: absent block defaults to `pin`; zero-touch-without-issuer fails; malformed tag fails (4 tests, green)

## 2. Machine-identity validation in node-registry FaaS

- [x] 2.1 Extend the `/admin/enrollment/start` payload to optionally carry a machine-identity JWT + injected policy (node-registry `EnrollmentStartRequest`)
- [x] 2.2 Implement OIDC verification in the FaaS: fetch `oidc_issuer` discovery + JWKS via `outbound-http`, verify RS256 signature (pure-Rust `rsa`/`sha2`/`base64`), `aud` (= `oidc_audience`), and `exp` with leeway; extract claims (incl. k8s namespace/serviceaccount). See `jwt.rs` + `zero_touch.rs`
- [x] 2.3 Implement `auto_approve_tags` matching (exact `key=value`); on full match, fetch a cluster-CA credential from the signer route and stage it on the session; on no match / invalid / unreachable token, fail closed to PIN
- [x] 2.4 Unit tests: tag match/mismatch, empty-tags never auto-approves, aud/exp enforcement, JWK selection (5 tests, green); auto-sign uses cert-manager (see §5)

## 3. Provenance + audit

- [x] 3.1 Add `approved_by: String` and `approval_tags: Vec<String>` to `EnrolledNode` (`#[serde(default)]` for existing records)
- [x] 3.2 Set `approved_by = "pin"` (PIN) or `"oidc:<subject>"` (auto) + `approval_tags`; persist via `kv-partition` (`record_approval_with_provenance`)
- [x] 3.3 Emit a security/audit line (`[AUDIT] …`, captured by the host log sink) on auto-approval and on auto-approve denial
- [~] 3.4 Test: provenance fields covered by the type API + unit tests; full persisted-record assertion deferred to the live e2e (needs a kv-partition host)

## 4. Enrollment client attaches machine identity

- [x] 4.1 `system-faas-enrollment` reads `identity_token_path` (wired from `TACHYON_ENROLLMENT_IDENTITY_TOKEN_PATH` in `supervisors.rs`) and attaches the JWT to `/admin/enrollment/start`; PIN behavior unchanged when absent
- [~] 4.2 Token-attach is covered structurally (compiles + existing enrollment tests pass); a dedicated mock-HTTP test was not added (the crate has no HTTP test harness)

## 5. k8s bootstrap deployment assets

- [x] 5.1 StatefulSet + headless Service already exist (`deploy-mesh.yaml`); `manifests/deploy-zero-touch.yaml` documents pointing `enrollment_endpoint` at the headless Service
- [x] 5.2 `deploy-zero-touch.yaml` adds the cluster-CA Secret mount (signer pods) at `/ca/cluster-ca.seed` and documents `trusted_signers` seeding
- [x] 5.3 Example `zero-touch` config block (`mode=both`, k8s issuer, SA `auto_approve_tags`) + a projected-SA-token volume + `TACHYON_ENROLLMENT_IDENTITY_TOKEN_PATH` env
- [x] 5.4 Genesis documented: seed pod self-approves from the mounted CA secret (or pre-sealed)

## 6. Verification

- [x] 6.1 `system-faas-node-registry` + `system-faas-cert-manager` build to `wasm32-wasip2`; `core-host` + `system-faas-enrollment` check; `cargo fmt --check` clean; clippy clean on the FaaS crates; node-registry 6 tests green
- [x] 6.2 Back-compat: `enrollment_block_absent_defaults_to_pin` (PIN-only manifest validates unchanged) + 3 more config tests, green
- [x] 6.3 E2E wired into CI: `.github/workflows/e2e-zero-touch.yml` (k3d) seals a zero-touch `integrity.lock` (`scripts/e2e/seal-zero-touch.js`), stands up a mock OIDC issuer from a generated fixture (`scripts/e2e/make-oidc-fixture.js`), deploys the mesh, and asserts the **hard security invariant** (no-token / unverifiable-token never auto-approves → PIN fallback). The full positive auto-approve is a best-effort step (continue-on-error) pending live validation of the signer-route module resolution. Both helper scripts verified locally; RS256 verify covered by the `jwt.rs` round-trip unit test
- [x] 6.5 Validate the two unknowns by test:
  - route→module resolution — deterministic tests in `routing_aliases.rs` (`injected_signer_route_resolves_to_cert_manager_module`, `signer_route_without_target_misresolves`); the seal script now sets an explicit `targets` so the signer route resolves to `system_faas_cert_manager.wasm`; the cluster E2E exercises it live as a hard gate.
  - credential — `system-faas-cert-manager` round-trip test proves the credential is a valid ed25519 CA signature (rejected by any other key). **Finding:** `tls_runtime` has no consumer of the enrolled credential today (`has_enrollment_credentials` only checks file existence), so there is no on-disk format contract to assert; format-alignment is deferred to a future real mTLS consumer (documented in design).
- [x] 6.4 `openspec validate zero-touch-node-enrollment --strict` passes

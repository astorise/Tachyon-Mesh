## Context

Existing enrollment is a human device-flow:
- A credential-less `core-host` boots in bootstrap mode and runs only `system-faas-enrollment`, which POSTs `/admin/enrollment/start` (its public key) to `enrollment_endpoint` over an outbound tunnel, then long-polls `/admin/enrollment/poll/{session_id}` ([enrollment/lib.rs](systems/system-faas-enrollment/src/lib.rs)).
- An operator enters the PIN in Studio against any active node; the ceremony in `system-faas-node-registry` (`enrollment.rs`: `EnrollmentSession`, `EnrollmentOutcome`, `deterministic_pin`, `node_id_from_public_key`) signs the CSR with the cluster CA and returns the cert down the tunnel. The approved node is persisted as `EnrolledNode` via the `node-registry` kv-partition table.
- After enrollment the node loads `system-faas-mesh-overlay` (heartbeat + capabilities) and `system-faas-gossip` converges config/membership (multi-master, admin-signed pull). This part is done and unchanged.

Config already has `enrollment_endpoint` and `trusted_signers`. There is no machine-identity path and no `auto_approve_tags`.

## Goals / Non-Goals

**Goals:**
- A pod can enroll with **no human step** when it presents a verifiable machine identity that matches policy.
- Keep the PIN path intact as the fallback (edge/NAT, or unmatched identity).
- Keep the ceremony inside `system-faas-node-registry`; `core-host` only forwards routes.
- Document a concrete, reproducible k8s bootstrap topology.

**Non-Goals:**
- Replacing the PIN flow.
- Changing the post-enrollment overlay/gossip convergence.
- Building a generic IdP; we validate JWTs against a configured OIDC issuer (k8s API server, or the homelab OIDC).
- A web UI for auto-approve policy (config-only in this change).

## Decisions

**D1 — Machine identity = projected ServiceAccount JWT, validated via OIDC.** Each pod mounts a short-lived, audience-scoped projected SA token (k8s `TokenRequest`). `system-faas-enrollment` attaches it to `/admin/enrollment/start`. The receiving node-registry FaaS fetches the issuer's JWKS (`oidc_issuer` + `/.well-known/openid-configuration`), verifies signature + `aud` + `exp`, and reads claims (`iss`, `sub`, `kubernetes.io` namespace/serviceaccount, or custom labels). Alternative — a shared bootstrap secret/token — rejected: a single shared secret is a weaker, non-revocable, non-attributable credential; per-pod SA tokens are revocable and auditable.

**D2 — Auto-approve is policy-gated, never network-gated.** Approval requires the validated claims to match every matcher in `auto_approve_tags` (e.g. `["namespace=tachyon", "serviceaccount=tachyon-node"]`). Matching a network range or "is in-cluster" alone is insufficient. No match → fall back to PIN. This mirrors the `trusted_signers` posture for config: identity, not location.

**D3 — Ceremony stays in the FaaS; OIDC verification too.** JWT/JWKS verification runs inside `system-faas-node-registry` (`control-plane-faas` world). The FaaS already reaches the network via `outbound-http` for JWKS fetch; `core-host` adds nothing but route forwarding. Rationale: keeps the trust decision in one auditable place and satisfies the architecture rule that enrollment logic lives in the FaaS.

**D4 — Config block shape.** `enrollment = { mode, oidc_issuer?, oidc_audience?, auto_approve_tags[] }`. `mode` defaults to `pin` (back-compat: existing manifests behave exactly as today). Validation: `zero-touch`/`both` require a non-empty `oidc_issuer`; `auto_approve_tags` entries must be `key=value`. The whole block is optional and `skip_serializing_if`-empty, so it never appears in PIN-only manifests.

**D5 — Provenance on the record.** `EnrolledNode` gains `approved_by: String` (`"pin:<operator-id>"` or `"oidc:<subject>"`) and `approval_tags: Vec<String>` (the matched matchers). Every approval (PIN or auto) emits an audit event. This makes "who/what let this node in" answerable after the fact.

**D6 — k8s bootstrap topology.**
- **StatefulSet + headless Service** → stable per-pod DNS (`tachyon-N.tachyon.<ns>.svc`).
- `enrollment_endpoint` = the headless Service (any ready peer answers `/admin/enrollment/start`), so a new pod needs no master.
- **Secrets**: cluster CA private key (mounted only on nodes allowed to sign), admin-signed `integrity.lock`, and the projected SA token (auto-mounted by k8s). `trusted_signers` is seeded from the CA/admin public key in the sealed config.
- **Genesis**: the seed pod (ordinal 0) is pre-sealed / self-approves from the mounted CA secret so the first node has authority without a peer to ask.

## Risks / Trade-offs

- **[Compromised SA token]** → tokens are short-lived, audience-scoped, and revocable; approval is also bounded by `auto_approve_tags`; all approvals are audited. A leaked token only enrolls within the policy's namespace/SA.
- **[OIDC issuer reachability]** → if JWKS fetch fails the FaaS denies auto-approve and falls back to PIN rather than failing open. Cache JWKS with a short TTL to tolerate blips.
- **[CA key exposure in-cluster]** → only signer-eligible pods mount the CA key; others enroll by asking a signer peer. Consider delegating signing to `system-faas-cert-manager` / an external CA (cert-manager/SPIRE) as a follow-up.
- **[Clock skew on `exp`/`nbf`]** → apply a small leeway window in verification.
- **[Back-compat]** → `mode` defaults to `pin`; absent `enrollment` block = today's behavior exactly.

## Migration Plan

1. Add the `enrollment` config block + validation (no behavior change while `mode = pin`).
2. Add JWT/JWKS verification + claim→tag matching + auto-sign branch in `system-faas-node-registry`; add `approved_by`/`approval_tags` to `EnrolledNode` (default empty for existing records).
3. Teach `system-faas-enrollment` to attach the projected SA token when present.
4. Ship k8s manifests (StatefulSet, headless Service, Secrets) + an example `zero-touch` `integrity.lock`.

Rollback: set `mode = pin` (or remove the `enrollment` block) — the machine-identity branch is never taken and the PIN flow is unchanged. No data migration (new record fields default empty).

## Resolved at apply time (Option B — delegate signing to a mesh signer)

- **Signer = `system-faas-cert-manager`.** node-registry, after OIDC+tag validation, calls a new signer route (`POST /internal/cert-manager/sign-node`, `{ nodePublicKey }`) over `outbound-http`, gets the credential bytes, and stages them on the session exactly like the PIN path stages `signed_certificate_hex`. The enrolling node's existing poll then receives them unchanged.
- **Credential = ed25519-signed token, not X.509.** `rcgen` is gated off wasm (cert-manager already mocks X.509 on wasm), so real X.509 signing is not available in-guest. The cluster trust model is already ed25519 (`trusted_signers` are ed25519 keys), so the signer issues an ed25519-signed node credential (`ed25519-dalek`, pure-Rust/wasm-safe), validated cryptographically by a round-trip unit test (sign→verify under the CA key).
  - **Finding (validation pass):** there is currently **no consumer** of the enrolled credential — `has_enrollment_credentials()` only checks the cert file *exists* (`is_file()`), and nothing in `core-host`/`tls_runtime` parses the enrolled-node credential into a TLS identity. So "the format `tls_runtime` expects" is **undefined today**; there is no contract to assert a format against. We therefore validate *cryptographic soundness* (the credential is a valid CA signature over the node key) and leave format-alignment to whenever a real mTLS consumer is built.
- **Route→module resolution (validated).** Module selection uses `route.targets[0].module` (else the path's last segment). The sealed signer route therefore carries an explicit `targets: [{ module: "system-faas-cert-manager" }]` so it resolves to `system_faas_cert_manager.wasm`; `routing_aliases.rs` asserts both the positive resolution and that a target-less route mis-resolves to `sign-node`.
- **CA key provisioning into cert-manager.** The cluster-CA ed25519 key is read from a mounted volume (cert-manager runs in `system-faas-guest`, which has `storage-broker`/volumes), with a generate-and-persist dev fallback. Only signer-eligible pods mount it.
- **Enrollment policy reaches the FaaS via the host.** node-registry does not receive the whole `IntegrityConfig`; `core-host`'s enrollment-forwarding handler injects the active `enrollment` policy (`oidc_issuer`, `oidc_audience`, `auto_approve_tags`, signer route) into the forwarded `/admin/enrollment/start` request so the FaaS evaluates policy without new global state.
- **JWT verification in-guest** uses pure-Rust `rsa` + `sha2` + base64 (RS256), JWKS fetched via `outbound-http` with a short cache. No `ring`/`jsonwebtoken` (not wasm-friendly).

## Open Questions

- Should signing be centralized to a single `cert-manager` FaaS instead of any CA-holding peer, to shrink CA-key blast radius? (Leaning yes as a follow-up; out of scope here.)
- Do we want `auto_approve_tags` to support claim globs (`team=*`) in v1, or exact `key=value` only? (Default: exact only in v1.)

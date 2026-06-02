## Why

Node enrollment today is PIN-based and human-driven: `system-faas-enrollment` opens an outbound tunnel and prints `Enter PIN in Tachyon-UI`, and an operator approves through Studio. That is the right model for edge/NAT nodes, but it does not scale to a multi-node cluster deployment (k8s/k3s) where pods are created and destroyed by autoscaling — no human can type a PIN per pod. The `device-flow-enrollment` spec already calls for declarative `auto_approve_tags` / zero-touch provisioning, but it is **not implemented**, and there is no documented bootstrap-endpoint discovery for the outbound tunnel in-cluster.

## What Changes

- **Machine-identity enrollment (new path in `system-faas-node-registry`)**: `/admin/enrollment/start` MAY carry a machine identity proof — a projected Kubernetes ServiceAccount JWT (audience-scoped) — alongside or instead of the human PIN. The FaaS validates the JWT against a configured OIDC issuer; if the token's claims match `auto_approve_tags`, it auto-signs the node CSR with the cluster CA and returns the certificate down the existing outbound tunnel, with **no human step**. The ceremony stays entirely inside the FaaS (`control-plane-faas` world); `core-host` keeps only route forwarding.
- **PIN remains the fallback**: when no token is presented, or claims do not match, enrollment falls back to the existing operator-PIN approval. PIN and zero-touch can coexist (`mode = both`).
- **Config surface (`IntegrityConfig`)**: a new optional `enrollment` block — `mode` (`pin` | `zero-touch` | `both`), `oidc_issuer`, `oidc_audience`, `auto_approve_tags` (list of `key=value` claim matchers). `enrollment_endpoint` (the outbound bootstrap URL) already exists. Validation rejects `zero-touch`/`both` without an `oidc_issuer`.
- **Provenance + audit**: the `EnrolledNode` record gains an `approved_by` field (`pin:<operator>` or `oidc:<subject>`) and the matched tags; every approval emits a security/audit event. Untrusted/unsigned tokens are rejected (same posture as `trusted_signers` for config updates).
- **k8s bootstrap topology (deployment, not code)**: a StatefulSet + headless Service gives each pod stable DNS; `enrollment_endpoint` points at the headless Service (or a seed subset); the cluster CA private key and admin-signed `integrity.lock` are mounted from k8s Secrets; `trusted_signers` is seeded from the CA public key; the seed/genesis pod self-approves from the mounted CA secret. Post-enrollment membership converges via the **already-implemented** `system-faas-mesh-overlay` heartbeats and `system-faas-gossip` — referenced, not changed.

## Capabilities

### New Capabilities

- `zero-touch-enrollment`: Automated, identity-gated node enrollment for cluster deployments — OIDC/JWT machine-identity validation, `auto_approve_tags` matching, the `enrollment` config block, and the k8s StatefulSet/headless-Service bootstrap topology.

### Modified Capabilities

- `device-flow-enrollment`: the declarative-provisioning requirement is made concrete (machine-identity proof + OIDC validation + auto-approve), and the PIN path is restated as the fallback when no matching machine identity is presented.
- `mesh-node-registry`: the enrollment-ceremony requirement gains the machine-identity/auto-approve branch (still inside the FaaS); the persistent registry record gains approval provenance (`approved_by`, matched tags).

## Impact

- **`systems/system-faas-node-registry/`** (`enrollment.rs`, `types.rs`, `lib.rs`): JWT validation, claim→tag matching, auto-sign branch, `approved_by`/tags on `EnrolledNode`, audit event.
- **`systems/system-faas-enrollment/`**: attach the machine-identity token (read from the projected SA token path) to `/admin/enrollment/start` when present.
- **`core-host/src/host_core/domain_types.rs`** + validation: the `enrollment` config block and its validation rule.
- **Deployment assets** (new, under the homelab/k8s manifests): StatefulSet, headless Service, Secrets (CA key, integrity.lock), example `enrollment` config.
- **Unchanged, referenced only**: `distributed-control-plane`, `p2p-mesh-overlay` (membership convergence after enrollment).

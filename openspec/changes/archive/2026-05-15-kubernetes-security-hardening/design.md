# Design: Kubernetes Security Hardening

## What Was Built

A production-grade hardened Kubernetes manifest for enterprise and regulated environments, layering identity isolation, Pod Security Standards (Restricted), and zero-trust NetworkPolicy onto the existing GPU homelab deployment.

### Task 1 — `manifests/deploy-gpu-homelab-hardened.yaml`

Single-file manifest with five components separated by `---`:

**ServiceAccount** (`tachyon-sa`)
- Dedicated identity, `automountServiceAccountToken: false` — no implicit RBAC grants.

**PersistentVolumeClaim** — unchanged from the base manifest (50 Gi, local-path).

**Deployment** — inherits all GPU nodeSelector/tolerations and probes from the base, adds:
- Pod security context: `runAsNonRoot: true`, `runAsUser/Group/fsGroup: 10001`, `seccompProfile: RuntimeDefault`
- Container security context: `allowPrivilegeEscalation: false`, `readOnlyRootFilesystem: true`, `capabilities.drop: [ALL]`
- `automountServiceAccountToken: false` at both Pod and Deployment spec level
- `emptyDir` volume mounted to `/tmp` — required because the root filesystem is read-only but the application needs a writable scratch area (Task 2 finding: `/tmp` and `/var/lib/tachyon/models` are the only writable paths needed; models PVC covers the latter)

**NetworkPolicy** (`tachyon-network-policy`) — zero-trust perimeter:
- Default-deny all ingress and egress
- Ingress: port 8080 for API/MCP traffic; port 8080 from `monitoring` namespace for Prometheus scraping
- Egress: UDP+TCP port 53 (DNS), TCP port 443 (Kubernetes API + external artifact fetch)

**ServiceMonitor** — unchanged from the base manifest.

### Task 2 — Temp Directory Compatibility

Audited `core-host` write paths:
- `/tmp` — used for transient scratch files during WASM compilation/staging; mounted as `emptyDir`
- `/var/lib/tachyon/models` — model cache; covered by the existing `tachyon-model-pvc` PVC
- `/app/integrity.lock` — read-only configmap mount; no writes needed

No additional `emptyDir` volumes are required beyond `/tmp`. All paths are either read-only or covered by existing mounts.

### Task 3 — README Update

Added a `> Enterprise / regulated environments:` callout block in the GPU/Homelab section immediately below the base manifest reference. Lists the security features and links to the hardened manifest.

## Files Changed
- `manifests/deploy-gpu-homelab-hardened.yaml` (new)
- `README.md`

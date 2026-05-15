# Proposal: Kubernetes Production Hardening

## Context
The T+5 audit cleared Tachyon-Mesh for a Release Candidate, but identified Kubernetes security posture as the final blocker for General Availability (GA). While `deploy-gpu-homelab.yaml` successfully schedules GPU workloads, it lacks the defensive layers required by enterprise Kubernetes administrators.

## Problem
1. **Privilege Escalation Risk:** The current pods run with default privileges, potentially as `root`, and can write to the root filesystem.
2. **Lateral Movement:** The absence of `NetworkPolicy` means if a Tachyon-Mesh pod is compromised, the attacker has unrestricted lateral access to the entire cluster network.
3. **Over-permissioned:** The deployment relies on the `default` ServiceAccount rather than a dedicated identity with least-privilege RBAC.

## Proposed Solution
Create `manifests/deploy-gpu-homelab-hardened.yaml` that implements strict security controls:
1. **Pod Security Standards (Restricted):** Enforce `runAsNonRoot: true`, `readOnlyRootFilesystem: true`, drop `ALL` Linux capabilities, and set `seccompProfile` to `RuntimeDefault`.
2. **Network Isolation:** Implement a Default-Deny `NetworkPolicy`, explicitly allowing only ingress on port 8080 (MCP/API) and Prometheus metrics scraping, while allowing egress only to essential services (DNS, Kubernetes API).
3. **Identity:** Create a dedicated `ServiceAccount` for Tachyon-Mesh.

## Impact
- **Compliance:** Passes automated security scanners (e.g., Kyverno, Trivy, OPA Gatekeeper) out-of-the-box.
- **GA Readiness:** Eliminates the very last technical blocker for an Enterprise Production release.
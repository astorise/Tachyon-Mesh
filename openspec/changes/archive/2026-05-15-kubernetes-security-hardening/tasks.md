# Implementation Tasks

- [x] **Task 1: Hardened Manifest Creation**
  - Create `manifests/deploy-gpu-homelab-hardened.yaml`.
  - Combine the `ServiceAccount`, the `Deployment` (with `securityContext`), the `Service`, the `PVC`, and the `NetworkPolicy` into a single file separated by `---`.

- [x] **Task 2: Temp Directory Compatibility**
  - Verify if `core-host` attempts to write to paths other than `/tmp` or `/var/lib/tachyon/models`. If so, ensure those paths are either configurable or mounted as `emptyDir` volumes to support `readOnlyRootFilesystem: true`.

- [x] **Task 3: Documentation Update**
  - Update `README.md` in the Deployment section to highlight the hardened manifest. 
  - Add text: *"For Enterprise and highly-regulated environments, use the hardened manifest which enforces strict Pod Security Standards (Restricted) and NetworkPolicies."*
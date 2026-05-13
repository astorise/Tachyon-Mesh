# Implementation Tasks

- [x] **Task 1: Workflow Creation**
  - Create `.github/workflows/e2e-k8s.yml` based on the specification.

- [x] **Task 2: Manifest Verification**
  - Ensure `manifests/deploy.yaml` includes a proper `/health` endpoint definition (or equivalent readiness probe) so `kubectl wait` works correctly.
  - Ensure labels (e.g., `app: tachyon-host`) match the workflow assertions.

- [x] **Task 3: Local Image Pipeline (Optional)**
  - If we want PRs to test the *unreleased* code, add a step to run `docker build -t tachyon:local .`, load it into the cluster, and patch the manifest to use `tachyon:local` and `imagePullPolicy: Never`.

- [x] **Task 4: Run & Validate**
  - Open a dummy PR modifying `deploy.yaml` to trigger the Action and verify the `vcluster` spins up and passes successfully.

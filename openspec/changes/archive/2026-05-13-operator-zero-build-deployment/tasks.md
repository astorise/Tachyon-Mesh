# Implementation Tasks

- [x] **Task 1: Create Downloader Script**
  - Write `scripts/get-tachyon.sh` to fetch artifacts from GitHub releases.
  - Test the script locally against an existing release (or a mocked release tag if none exists yet).

- [x] **Task 2: Release Workflow Alignment**
  - Verify that `.github/workflows/release.yml` accurately produces the tarballs (`tachyon-mesh-VERSION-OS-ARCH.tar.gz`) expected by the download script, containing at minimum `core-host` and `tachyon-mcp`.

- [x] **Task 3: README Split**
  - Update `README.md` to clearly differentiate "Path A: Operators" (using the new script and `manifests/deploy.yaml`) and "Path B: Contributors" (using `setup.sh`).

- [x] **Task 4: K8s Manifest Review**
  - Do a quick review of `manifests/deploy.yaml` to ensure the container image tag points to `latest` or the correct semantic version, so the one-liner `kubectl apply` works out of the box.

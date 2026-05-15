# Implementation Tasks

- [x] **Task 1: GitHub Actions Update**
  - Edit `.github/workflows/release.yml`.
  - Add the `id-token: write` permission.
  - Integrate `anchore/sbom-action` to generate the SPDX JSON.
  - Add steps to calculate the SHA256 checksums and sign the binaries using `cosign sign-blob`.
  - Ensure the release step uploads all auxiliary files (`.sha256`, `.bundle`, `-sbom.spdx.json`).

- [x] **Task 2: Hardening `get-tachyon.sh`**
  - Add the `curl` call to fetch the `.sha256` file matching the architecture.
  - Implement `sha256sum -c` to validate the downloaded tarball before extraction.

- [x] **Task 3: Hardening `get-tachyon.ps1`**
  - Add the `Invoke-WebRequest` to fetch the Windows `.sha256` file.
  - Implement the `Get-FileHash` check to validate the downloaded `.zip`.

- [x] **Task 4: README Update**
  - Add a brief note in the "Zero-Build" section of the `README.md` explicitly stating: *(Binaries are automatically verified via SHA-256 upon download. Signatures and SBOMs are available in the GitHub Releases).*
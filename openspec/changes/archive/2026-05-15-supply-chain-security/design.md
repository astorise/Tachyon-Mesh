# Design: Supply-Chain Security

## What Was Built

End-to-end supply-chain hardening: every release artifact is checksummed, signed, and accompanied by an SPDX SBOM, and both installer scripts verify integrity before extraction.

### Task 1 — GitHub Actions (`release.yml`)
- Added `id-token: write` permission to `publish-server-binaries` to enable keyless OIDC signing via Sigstore.
- `Package archive` step now writes the filename to `$GITHUB_OUTPUT` (`steps.pkg.outputs.tarball`) instead of `$GITHUB_ENV` — eliminates static-analysis false positives from the GitHub Actions linter.
- New `Calculate SHA-256 checksum` step: runs `sha256sum` and writes `<archive>.sha256` alongside the binary.
- New `Install cosign` + `Sign binary archive` steps (skipped on Windows runners): keyless `cosign sign-blob --yes` produces a `<archive>.bundle` Rekor transparency-log entry.
- New `Generate SBOM` step (Linux runner only): `cargo sbom --output-format spdx_json_2_3` produces `<archive>-sbom.spdx.json`.
- `Upload archive` step now uploads all four artifacts: tarball, `.sha256`, `.bundle`, `-sbom.spdx.json`.

### Task 2 — `scripts/get-tachyon.sh`
- After download, fetches `<tarball>.sha256` from the release (fails loudly on HTTP error).
- Runs `sha256sum -c --status` in a subshell; aborts with an explicit tamper warning on mismatch.
- Checksum verified before extraction — the tarball is never unpacked if corrupt.

### Task 3 — `scripts/get-tachyon.ps1`
- After download, fetches `<zip>.sha256` via `Invoke-WebRequest` into a temp file.
- Compares `Get-FileHash -Algorithm SHA256` against the expected hash (case-insensitive).
- Deletes the downloaded zip and fails with both hashes printed if there is a mismatch.
- Temp checksum file is always cleaned up.

### Task 4 — `README.md`
- Added a `> Security:` callout in the Zero-Build section documenting SHA-256 auto-verification and the availability of cosign bundles + SBOMs in GitHub Releases.

## Files Changed
- `.github/workflows/release.yml`
- `scripts/get-tachyon.sh`
- `scripts/get-tachyon.ps1`
- `README.md`

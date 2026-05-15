# Proposal: Supply Chain Security (SBOM, Sigstore, Checksums)

## Context
The T+5 release audit cleared Tachyon-Mesh for a Release Candidate (`v0.x-rc1`) but flagged the software supply chain as the primary blocker for a General Availability (GA) production release. While we successfully build and distribute cross-platform binaries via GitHub Actions, we lack verifiable cryptographic attestation.

## Problem
1. **Unverifiable Artifacts:** End-users running the `get-tachyon.sh` or `.ps1` zero-build scripts blindly download and execute binaries without verifying their integrity via SHA-256 checksums.
2. **Missing Signatures:** Enterprise security teams require binary provenance. Our GitHub Releases lack signatures verifying that the binaries were actually built by our CI runner and not tampered with.
3. **Black-box Dependencies:** The project does not publish a Software Bill of Materials (SBOM), making it impossible for automated vulnerability scanners to audit the specific crates and npm packages bundled inside the final `.tar.gz` or `.zip`.

## Proposed Solution
1. **SBOM Generation:** Integrate `cargo-sbom` (or `syft`) into `.github/workflows/release.yml` to generate an SPDX JSON file for every release artifact.
2. **Keyless Signing (Sigstore):** Utilize `cosign` within the GitHub Action to perform keyless signing of the release artifacts using the GitHub OIDC identity.
3. **Checksum Verification in Installers:** Update the onboarding scripts (`get-tachyon.sh` and `get-tachyon.ps1`) to download the `sha256sums.txt` file from the release, compute the local hash, and abort execution if they do not match.

## Impact
- Elevates Tachyon-Mesh to Enterprise/Production-Grade security standards.
- Secures the "Zero-Build" onboarding path against man-in-the-middle (MITM) attacks and compromised artifact registries.
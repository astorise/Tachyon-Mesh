# Technical Specification: Secure Release Pipeline

## 1. Release Workflow Enhancements (`.github/workflows/release.yml`)
Update the primary release workflow to include SBOM generation, checksum calculation, and keyless signing.

### Add Permissions
To use Sigstore/cosign keyless signing, the workflow requires `id-token: write`.
```yaml
permissions:
  contents: write # For creating releases
  id-token: write # For cosign keyless signing
  packages: write # If pushing to GHCR
```

### Add Steps (After Build & Package)
Insert these steps into the release matrix job:

```yaml
      - name: Generate SBOM (SPDX)
        uses: anchore/sbom-action@v0
        with:
          path: ./
          format: spdx-json
          output-file: tachyon-mesh-${{ matrix.target }}-sbom.spdx.json

      - name: Calculate Checksums
        run: |
          sha256sum ${{ matrix.artifact_name }} > ${{ matrix.artifact_name }}.sha256
          # Note: On Windows runner, use Get-FileHash or powershell equivalent

      - name: Install Cosign
        uses: sigstore/cosign-installer@v3.5.0

      - name: Sign Release Artifacts
        run: |
          cosign sign-blob --yes ${{ matrix.artifact_name }} \
            --bundle ${{ matrix.artifact_name }}.bundle
```

*Ensure all these new artifacts (SBOM, `.sha256`, `.bundle`) are uploaded to the GitHub Release alongside the primary `.tar.gz` or `.zip`.*
# Technical Specification: Windows Release

## 1. Update `release.yml`
Modify `.github/workflows/release.yml` to add the Windows matrix target.

```yaml
jobs:
  build-release:
    strategy:
      matrix:
        include:
          - os: ubuntu-latest
            target: x86_64-unknown-linux-gnu
            artifact_name: tachyon-mesh-linux-amd64.tar.gz
          - os: macos-latest
            target: aarch64-apple-darwin
            artifact_name: tachyon-mesh-darwin-arm64.tar.gz
          # NEW: Windows Target
          - os: windows-latest
            target: x86_64-pc-windows-msvc
            artifact_name: tachyon-mesh-windows-amd64.zip
```

*Ensure the compression step handles `.zip` correctly for the Windows runner instead of `tar.gz`.*
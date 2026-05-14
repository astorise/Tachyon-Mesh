# WIT OCI Registry Publication Specifications

## 1. CI/CD Workflow Modifications
Codex must append a new job `publish-wit-oci` to the release pipeline (e.g., `.github/workflows/publish-sdks.yml` which triggers on `release: published`).

**Required Workflow Steps Definition:**
```yaml
  publish-wit-oci:
    name: Publish WIT to GHCR
    runs-on: ubuntu-latest
    permissions:
      contents: read
      packages: write
    steps:
      - name: Checkout repository
        uses: actions/checkout@v4

      - name: Install wkg (Wasm Package Tools)
        run: cargo install wkg

      - name: Log in to GitHub Container Registry
        uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}

      - name: Package and Publish WIT Contracts
        run: |
          # Extract the semantic version from the Git tag (stripping the 'v' prefix)
          # Example: refs/tags/v0.9.0-rc.1 -> 0.9.0-rc.1
          VERSION=${GITHUB_REF#refs/tags/v}
          REGISTRY_URL="ghcr.io/${{ github.repository_owner }}/tachyon-mesh-wit"
          
          # Publish using wkg
          wkg publish --registry ghcr.io --namespace ${{ github.repository_owner }} --package tachyon-mesh-wit ./wit
```
*(Note to Codex: Ensure the exact syntax for `wkg publish` matches the latest Bytecode Alliance CLI specifications for OCI publication, adjusting arguments if necessary).*

## 2. Developer Documentation (Guest Consumption)
Codex must update the documentation (e.g., `faas-sdk/README.md` or the root `README.md`) to show FaaS developers how to cleanly import the WIT contracts without local files.

**Documentation Snippet Example (Using RC versions):**
```markdown
### Using Tachyon WIT Interfaces in your FaaS

Tachyon publishes its WebAssembly interfaces as OCI artifacts to GHCR. You do not need to copy `.wit` files locally. Instead, add the following to your component's `Cargo.toml`:

```toml
[package.metadata.component.dependencies]
"tachyon:mesh" = "oci://ghcr.io/astorise/tachyon-mesh-wit:0.9.0-rc.1"
"tachyon:ai" = "oci://ghcr.io/astorise/tachyon-mesh-wit:0.9.0-rc.1"
```
Ensure you are using `cargo-component` to build your project, which will automatically fetch and resolve these OCI dependencies during compilation.
```
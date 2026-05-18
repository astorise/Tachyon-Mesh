# Tachyon FaaS SDK

This directory contains the FaaS SDK crate for building Rust guest functions that run on Tachyon Mesh.

## Quick Start

```toml
# Cargo.toml
[dependencies]
tachyon-faas-sdk = "1.1.0"
```

```rust
use tachyon_sdk::prelude::*;

#[tachyon_function]
fn handle_request(req: Request) -> Response {
    Response::builder()
        .status(200)
        .body("Hello from Tachyon Mesh!")
        .build()
}
```

Build targeting `wasm32-wasip2`:

```bash
cargo build --target wasm32-wasip2 --release
```

---

## WIT Contracts via OCI (cargo-component)

If you prefer to use [`cargo-component`](https://github.com/bytecodealliance/cargo-component) directly against the WIT interface rather than the pre-built SDK crate, Tachyon publishes its WIT contracts as OCI artifacts to GitHub Container Registry.

### Setup

1. Install `cargo-component`:
   ```bash
   cargo install cargo-component --locked
   ```

2. Add the dependency to your `Cargo.toml`:
   ```toml
   [package.metadata.component.dependencies]
   "tachyon:mesh" = { registry = "oci", package = "ghcr.io/astorise/tachyon-mesh-wit", version = "1.1.0" }
   ```

3. Build your component:
   ```bash
   cargo component build --target wasm32-wasip2 --release
   ```

`cargo-component` fetches the WIT artifact from GHCR automatically — no local `.wit` files needed.

### Available interfaces

| Package | Interfaces |
|---|---|
| `tachyon:mesh` | `handler`, `kv-partition`, `graph`, `vector`, `training`, `outbound-http`, `secrets-vault`, … |

### Pinning a specific version

Replace `1.1.0` with the release tag you want to target, e.g. `0.9.0-rc.1`. To list all published versions:

```bash
wkg list ghcr.io/astorise/tachyon-mesh-wit
```

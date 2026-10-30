# Tachyon FaaS SDK

This directory contains `faas-sdk`, a small, optional, workspace-internal
proc-macro crate exposing `#[faas_sdk::faas_handler]` -- applying it to a
guest's `handle_request` wires up JSON-structured `tracing` output to
stdout automatically. It is **not** published to crates.io and is not
required to write a guest function.

For the `kv-partition` interface's typed `Table` client, see the real,
published `tachyon-faas-sdk` crate under [`sdk/rust/`](../sdk/rust/)
instead -- despite the similar name, it is a different crate from this
directory's `faas-sdk`.

## Quick Start

A Tachyon Mesh guest implements the `handler` world's `Guest` trait,
generated directly from the published WIT contract with
[`wit-bindgen`](https://github.com/bytecodealliance/wit-bindgen) (see
[`examples/guest-example`](../examples/guest-example/) for the full,
CI-built reference):

```toml
# Cargo.toml
[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.46"
```

```rust
mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "wit/tachyon.wit", // a local copy of the WIT contract -- see the
        world: "faas-guest",     // "WIT Contracts via OCI" section below for
    });                          // fetching one without vendoring it by hand

    export!(Component);
}

struct Component;

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        let body = if req.body.is_empty() {
            b"Hello from Tachyon Mesh!".to_vec()
        } else {
            req.body
        };
        bindings::exports::tachyon::mesh::handler::Response {
            status: 200,
            headers: vec![],
            body,
            trailers: vec![],
        }
    }
}
```

`Request`/`Response` are plain records generated from the `handler`
interface in `wit/tachyon.wit` -- construct them as ordinary structs, no
builder exists. Build targeting `wasm32-wasip2`:

```bash
cargo build --target wasm32-wasip2 --release
```

Need the `kv-partition` interface too (persistent key-value storage)?
Add `tachyon-faas-sdk` from [`sdk/rust/`](../sdk/rust/) as a second
dependency and import `tachyon_faas_sdk::ai::kv_partition::Table`
alongside the `handler` bindings above.

Want this directory's optional JSON-structured `tracing` output for free?
Depend on `faas-sdk` from a workspace member (it is not published
externally) and annotate `handle_request` with `#[faas_sdk::faas_handler]`
-- see this crate's own `src/lib.rs` for exactly what it injects.

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
   "tachyon:mesh" = { registry = "oci", package = "ghcr.io/astorise/tachyon-mesh-wit", version = "1.0.0" }
   ```
   This `version` is the git release tag `publish-wit-oci` published the artifact from (e.g. `v1.0.0` → `1.0.0`), **not** `wit/tachyon.wit`'s own internal `package tachyon:mesh@2.0.0;` declaration — that's a separate WIT ABI/interface-stability version that moves independently on breaking interface changes.

3. Build your component:
   ```bash
   cargo component build --target wasm32-wasip2 --release
   ```

`cargo-component` fetches the WIT artifact from GHCR automatically — no local `.wit` files needed.

### Available interfaces

| Package | Interfaces |
|---|---|
| `tachyon:mesh` | `handler`, `kv-partition`, `graph`, `vector`, `training`, `outbound-http`, `secrets-vault`, … |

### Outbound HTTP

Use the `tachyon:mesh/outbound-http` WIT import for guest-initiated HTTP calls. The
older `MESH_FETCH:<url>` stdout/body command is no longer supported. Guests that
need to call HTTP targets must import `tachyon:mesh/outbound-http`.

### Pinning a specific version

Replace `1.0.0` with the release tag you want to target, e.g. `0.9.0-rc.1`. To list all published versions:

```bash
wkg list ghcr.io/astorise/tachyon-mesh-wit
```

# Design: Embedded Core Store

## Summary

Introduce an embedded `redb` database inside `core-host` to persist internal
runtime state that must survive restarts without relying on an external cache
or loose filesystem artifacts.

## Tables

The host stores three categories of control-plane data:

- `cwasm_cache`: precompiled Wasmtime artifacts keyed by the source wasm hash
  and engine scope
- `tls_certs`: serialized certificate bundles issued or loaded for native TLS
- `hibernation_state`: serialized RAM-volume snapshots keyed by managed volume
  identifier

## Runtime Integration

The database is opened during host startup and shared through an `Arc`. Request
execution paths that already run on blocking worker threads may read and write
 the compiled artifact cache directly, while async call sites use
`tokio::task::spawn_blocking` when they must touch `redb`.

## Module Cache

Component and legacy module loads read the guest artifact bytes, hash them, and
look up a precompiled payload in `cwasm_cache`. Cache misses compile through the
current Wasmtime engine and immediately persist the resulting bytes for future
deserialization.

## TLS Persistence

Native TLS loads certificate bundles from `tls_certs` before falling back to the
filesystem cache written by `system-faas-cert-manager`. Provisioned or migrated
bundles are written back into `redb`, and startup primes any already-known
domains into the in-memory TLS config cache.

## Hibernation Persistence

RAM volume hibernation snapshots are serialized into a compact byte payload and
stored in `hibernation_state`. Restoring a hibernated volume recreates the
directory tree from the database payload and clears the stored snapshot entry
once the restore completes.

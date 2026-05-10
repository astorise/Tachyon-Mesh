# Proposal: WASM Artifact Sync and Trusted-Signer Open-Mode Semantics

## Why

Two independent regressions blocked the full core-host test suite after the
IAM, topology, and bundle-pipeline deliveries:

### 1 — WASM artifact drift

Seventeen WASM guest components had been compiled against an older WIT revision.
The `tachyon:mesh/handler@1.0.0` export, the `tachyon:mesh/outbound-http`,
`tachyon:mesh/telemetry-reader`, `tachyon:mesh/scaling-metrics`,
`tachyon:mesh/storage-broker`, and `tachyon:mesh/bridge-controller` import
signatures all changed during the WIT semantic-versioning enforcement deliveries
but the pre-compiled `.wasm` files in `target/wasm32-wasip2/release/` were
never rebuilt. Any test that instantiated these guests would fail at
`component imports instance … but a matching implementation was not found in the linker`.

### 2 — Trusted-signer semantics regression

The trusted-signer verification check introduced in `2026-05-09-host-signed-bundles`
applied the trust restriction unconditionally — even when the `trusted_signers`
slice passed to `verify_integrity_payload_with_trusted` was empty. This caused:

- **Boot-time loads** (`load_integrity_config_from_manifest_path`) to accept
  only the embedded boot key, breaking the post-bundle-apply boot where the
  manifest is now signed by the host key.
- **Batch-job execution** (`execute_batch_target_from_manifest`) to fail for
  the same reason.
- **Hot-reload tests** to fail because the running config starts with an empty
  `trusted_signers` list; any test key (non-embedded) was rejected even when
  the signature itself was cryptographically valid.
- **Schema-validation tests** to emit a "signer not trusted" error instead of
  the expected "schema violation" error, because the trust check ran before
  payload parsing.

## What Changes

### `core-host/src/host_core/integrity_config.rs`

`verify_integrity_payload_with_trusted` now gates the trust check on
`!trusted_signers.is_empty()`:

- **Empty list → open mode**: only the cryptographic signature is verified.
  This is the correct behaviour for local paths (boot, batch jobs) where
  filesystem access already implies physical-layer trust.
- **Non-empty list → restricted mode**: the signing key must be the embedded
  boot key or appear in the explicit `trusted_signers` set. This enforces
  cluster-level trust for manifests pushed over the network (hot-reload,
  admin manifest update).

### WASM artifacts rebuilt

All seventeen stale guests rebuilt with `--target wasm32-wasip2 --release`:

| Crate | Root cause |
|---|---|
| `system-faas-logger` | `tachyon:mesh/handler@1.0.0` export renamed |
| `system-faas-metering` | same |
| `system-faas-bridge` | `tachyon:mesh/bridge-controller` import signature change |
| `system-faas-sqs` | `tachyon:mesh/outbound-http` import signature change |
| `system-faas-cdc` | same |
| `system-faas-gateway` | same |
| `system-faas-s3-proxy` | same |
| `system-faas-storage-broker` | `tachyon:mesh/storage-broker` import signature change |
| `system-faas-buffer` | `tachyon:mesh/telemetry-reader` import signature change |
| `system-faas-k8s-scaler` | `tachyon:mesh/scaling-metrics` import signature change |
| `system-faas-prom` | same |
| `system-faas-keda` | same |
| `guest-flaky` | `tachyon:mesh/handler@1.0.0` export renamed |
| `guest-grpc` | same |
| `guest-udp-echo` | `tachyon:mesh/udp-handler@1.0.0` export renamed |
| `guest-voip-gate` | `tachyon:mesh/bridge-controller` import signature change |
| `guest-volume` | `tachyon:mesh/handler@1.0.0` export renamed |

## Result

210/210 core-host tests pass. The three categories of previously-broken tests
are restored:
- WASM instantiation tests (17 guests)
- Hot-reload and batch-execution tests (trusted-signer open mode)
- Schema-validation tests (error order restored)

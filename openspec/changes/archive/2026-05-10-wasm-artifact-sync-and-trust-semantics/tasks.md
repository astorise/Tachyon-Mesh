# Tasks

## WASM artifact sync

- [x] Rebuild `system-faas-logger` and `system-faas-metering` (handler export)
- [x] Rebuild `system-faas-bridge` (bridge-controller import)
- [x] Rebuild `system-faas-sqs`, `system-faas-cdc`, `system-faas-gateway`, `system-faas-s3-proxy` (outbound-http import)
- [x] Rebuild `system-faas-storage-broker` (storage-broker import)
- [x] Rebuild `system-faas-buffer`, `system-faas-prom`, `system-faas-k8s-scaler`, `system-faas-keda` (telemetry-reader / scaling-metrics imports)
- [x] Rebuild `guest-flaky`, `guest-grpc`, `guest-volume` (handler export)
- [x] Rebuild `guest-udp-echo` (udp-handler export)
- [x] Rebuild `guest-voip-gate` (bridge-controller import)

## Trusted-signer semantics

- [x] Gate trust check on `!trusted_signers.is_empty()` in `verify_integrity_payload_with_trusted`
- [x] Verify boot-time path (`load_integrity_config_from_manifest_path`) unaffected
- [x] Verify `execute_batch_target_from_manifest` unaffected
- [x] Verify hot-reload path (`reload_runtime_from_disk`) still enforces trust when signers are configured
- [x] All 210 core-host tests pass

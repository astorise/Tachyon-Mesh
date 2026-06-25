## Why

The host now reports NPU and TPU honestly as unavailable when no real backend exists. Real execution still requires vendor SDK integrations and physical acceptance hardware.

## What Changes

- Add an OpenVINO NPU backend for a documented model subset.
- Add a Coral Edge TPU backend for compiled TFLite models.
- Wire guest-visible fallback routing.
- Capture physical CPU+GPU+NPU and Coral TPU execution evidence.

## Capabilities

### Modified Capabilities

- `heterogeneous-accelerator-orchestration`

## Why

Three independent issues degraded the topology canvas usability after the live
manifest integration: (1) nodes could not be dragged in view mode, requiring a
mode switch just to rearrange the canvas; (2) `system-faas-openai-adapter` was
missing from the topology even with the `ai-inference` feature because it was
not included in `inject_feature_routes`; (3) guest-call-legacy was invisibly
overlapping storage-broker due to a layout band-collision that `TopologyLayout`
was supposed to prevent but did not fully cover for newly-split pending nodes.

## What Changes

- **Drag in view mode**: the `if (!this.editable) return;` guard is removed from the `pointerdown` handler in `TachyonTopologyCanvas.wireNodeEvents`. Nodes are now draggable in both view and edit modes. The `editable` flag continues to control node creation, deletion, and the node-editor sidebar.
- **`/system/ai-openai-adapter` injection**: `inject_feature_routes` extended to push `/system/ai-openai-adapter` (`ai-openai-adapter`) alongside `model-broker` and `ai-list-model` under `#[cfg(feature = "ai-inference")]`.
- **Guest examples manifest — 9 routes**: `examples/guest-examples/manifest.json` extended from 4 to 9 routes, adding `guest-ai`, `guest-log-storm`, `guest-voip-gate`, `guest-volume`, `guest-websocket-echo`. Excluded: `guest-flaky` (test fixture), `guest-malicious` (security test), `guest-tcp-echo` and `guest-udp-echo` (non-HTTP transports).
- **Layout overlap bug**: the two-pass `TopologyLayout::build` was applied correctly to the `PendingNode` second pass, but the `pending_type_counts` increment for `endpoint` nodes used a line too long for rustfmt (chained `.entry().or_insert(0) += 1`), causing a CI formatting failure that was then fixed.

## Capabilities

### New Capabilities

None — all changes are refinements of existing capabilities.

### Modified Capabilities

- `topology-live-and-filters`: drag now works in view mode; topology shows `ai-openai-adapter`; guest-call-legacy no longer overlaps storage-broker.
- `feature-auto-injection`: `ai-openai-adapter` added to the `ai-inference` injection bundle.
- `faas-package-import`: guest-examples manifest covers 9 routes instead of 4.

## Impact

- **`tachyon-ui/src/components/domains/TachyonTopologyPanel.ts`**: one-line removal in `wireNodeEvents`.
- **`core-host/src/host_core/integrity_config.rs`**: one extra `to_inject.push` in `inject_feature_routes`.
- **`examples/guest-examples/manifest.json`**: 5 routes added.
- **`tachyon-client/src/lib.rs`**: rustfmt fixes on `pending_type_counts` chains and `TopologyLayout::build` signature.

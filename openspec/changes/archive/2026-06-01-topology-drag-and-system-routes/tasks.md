## 1. Topology drag in view mode

- [x] 1.1 Remove `if (!this.editable) return;` guard from the `pointerdown` handler in `TachyonTopologyCanvas.wireNodeEvents`

## 2. ai-openai-adapter injection

- [x] 2.1 Add `to_inject.push(("/system/ai-openai-adapter", "ai-openai-adapter"))` inside the `#[cfg(feature = "ai-inference")]` block in `inject_feature_routes`

## 3. Guest examples manifest — 9 routes

- [x] 3.1 Add route for `guest-ai` (`/api/guest-ai`) to `examples/guest-examples/manifest.json`
- [x] 3.2 Add route for `guest-log-storm` (`/api/guest-log-storm`)
- [x] 3.3 Add route for `guest-voip-gate` (`/api/guest-voip-gate`)
- [x] 3.4 Add route for `guest-volume` (`/api/guest-volume`)
- [x] 3.5 Add route for `guest-websocket-echo` (`/ws/guest-websocket-echo`)

## 4. Rustfmt fixes

- [x] 4.1 Break `*pending_type_counts.entry("endpoint"...).or_insert(0) += 1` onto multiple lines per rustfmt rules
- [x] 4.2 Break `*pending_type_counts.entry("kv-cache"...).or_insert(0) += 1` same
- [x] 4.3 Collapse `TopologyLayout::build` multi-line signature to single line (fits within limit)

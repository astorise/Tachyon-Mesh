## 1. File-picker prerequisite

- [x] 1.1 Add `tauri-plugin-dialog` to `tachyon-ui/Cargo.toml` and register it (`.plugin(tauri_plugin_dialog::init())`) in the builder in `tachyon-ui/src/main.rs`
- [x] 1.2 Add an app command `pick_model_file() -> Result<Option<String>, String>` using the plugin's Rust API (native open-file dialog, single file, via a oneshot channel), and register it in `generate_handler!`
- [x] 1.3 No capabilities file / JS plugin dependency needed — `pick_model_file` is an app command (consistent with every other command in this app; avoids introducing capability scaffolding)

## 2. Model-upload panel component

- [x] 2.1 Create `tachyon-ui/src/components/domains/TachyonModelUploadPanel.ts` as a `<tachyon-model-upload-panel>` custom element extending `TachyonConfigDashboard`
- [x] 2.2 File selection via the `pick_model_file` app command (Tauri native dialog); store the chosen path (no `<input type=file>`)
- [x] 2.3 Invoke `push_large_model` through `resilientInvoke` (the privileged wrapper in `utils/network.ts`) — never a bare core `invoke`; relies on the existing step-up entry
- [x] 2.4 Subscribe to the `upload_progress` event while uploading; render a progress bar; disable the upload control during upload; unlisten on completion/failure
- [x] 2.5 On success show the returned asset ref + a hint that the model will appear in `/v1/models`; on failure show the translated backend error and re-enable the control
- [x] 2.6 `idle → uploading → success|error` state machine with network/event logic in separately-callable methods for testability

## 3. Wiring + i18n

- [x] 3.1 Mount `<tachyon-model-upload-panel>` in the AI view — the active AI surface is the `<tachyon-ai-panel>` custom element (route `ai`, gated `hasAi` in `ComponentRegistry.ts`); the `aiOrchestration.ts` string template is unused, so the panel is embedded in `TachyonAIPanel.render()` (import added for registration)
- [x] 3.2 Add i18n strings (`ai.upload.*`) for the panel in `tachyon-ui/src/utils/i18n.ts` (en + fr)

## 4. Tests

- [x] 4.1 Unit test the panel's state transitions and that it routes through `resilientInvoke` — asserts core `invoke` is never called
- [x] 4.2 Progress handling: an `upload_progress` event updates the bar; success renders the asset ref + registry hint; cancel is a no-op; error renders the translated message and re-enables the control
- [x] 4.3 Extend the i18n key-coverage test for the new `ai.upload.*` strings (en + fr)

## 5. Verification

- [x] 5.1 `tachyon-ui` green: `cargo check -p tachyon-ui` (plugin + command compile), `tsc --noEmit` clean, full `vitest run` 115/115 passing
- [~] 5.2 Manual smoke (live node with `model-broker` active): select a file → step-up → progress → success → model in `GET /v1/models` — requires a running node; not executable in this environment
- [x] 5.3 `openspec validate ai-model-upload-ui --strict` passes

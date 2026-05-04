# Execution Tasks for Codex

- [x] **Task 1 (Rust Backend)**: Modify `tachyon-ui/src/main.rs` to include the `apply_configuration` Tauri command as designed, and register it in the `invoke_handler`. Ensure `serde_json::Value` is imported correctly.
- [x] **Task 2 (Frontend JS)**: Modify `tachyon-ui/src/controllers/routingController.ts` to replace the `console.log` with a real `__TAURI__.core.invoke` call to the `apply_configuration` backend endpoint.
- [x] **Task 3 (OpenSpec)**: Create the `specs/tauri-configurator/spec.md` delta to formalize that all frontend intents must cross the Tauri IPC boundary to be strictly validated by Rust.

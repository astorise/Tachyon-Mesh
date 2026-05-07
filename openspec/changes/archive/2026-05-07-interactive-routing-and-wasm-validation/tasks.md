# Execution Tasks for Codex

- [x] **Task 1 (Frontend HTML)**: Overwrite `tachyon-ui/src/views/routing.ts` with the interactive form template provided in `design.md`.
- [x] **Task 2 (Frontend Logic)**: Update `tachyon-ui/src/controllers/routingController.ts` to replace the `readText` DOM scraping method with the `getValue` method that reads `.value` from the new `<input>` and `<select>` elements as specified in `design.md`.
- [x] **Task 3 (Backend Rust)**: Update `tachyon-ui/src/main.rs`. Implement the strict Serde structs (`TrafficConfig`, `TrafficSpec`, etc.) and refactor the `apply_configuration` command to deserialize the payload against these structs. Ensure it returns a failed `ApiResponse` (not an `Err`) if deserialization fails, so the UI can show the red toast.
- [x] **Task 4 (OpenSpec)**: Update `specs/tauri-configurator/spec.md` to formally require strict Rust/Serde validation of UI payloads before they are pushed to the data-plane.

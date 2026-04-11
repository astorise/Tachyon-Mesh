# Tasks: Change 061 Implementation

**Agent Instruction:** Do not alter the UI design. Your job is purely tooling setup and Rust-to-TypeScript integration.

## [TASK-1] Frontend Dependencies
- [x] Delete the old `dist` folder inside `tachyon-cli` if it exists.
- [x] Initialize Vite inside `tachyon-cli`. Ensure `package.json` includes `vite`, `typescript`, `tailwindcss`, `postcss`, `autoprefixer`, `gsap`, and `@tauri-apps/api`.
- [x] Create `vite.config.ts` with the Vite dev server pinned to port `5173`.
- [x] Create `postcss.config.js` with Tailwind CSS and Autoprefixer plugins.

## [TASK-2] Asset Injection
- [x] Use the provided code from the Product Owner to create `tachyon-cli/index.html`.
- [x] Use the provided code to create `tachyon-cli/tailwind.config.js`.
- [x] Use the provided code to create `tachyon-cli/src/style.css`.
- [x] Use the provided code to create `tachyon-cli/src/main.ts`.

## [TASK-3] Tauri Integration
- [x] Open `tachyon-cli/tauri.conf.json` and update the `build` configuration to use the repository’s Tauri v2 keys for Vite (`beforeDevCommand`, `beforeBuildCommand`, `devUrl`, `frontendDist`).
- [x] Open the existing Rust entrypoint in `tachyon-cli/src/lib.rs`.
- [x] Create a Tauri command: `#[tauri::command] fn get_engine_status() -> String { "42".into() }`.
- [x] Register the command in the `tauri::Builder::default().invoke_handler(tauri::generate_handler![get_engine_status])` pipeline and keep GUI launches from exiting immediately.

## Validation Step
- [x] Run `npm install` inside `tachyon-cli`.
- [x] Run `npm run build` inside `tachyon-cli` and ensure Vite compiles successfully without TypeScript errors.
- [x] Run `cargo build -p tachyon-cli`.
- [x] Run `npm run tauri build` inside `tachyon-cli` and ensure Tauri builds the final binary.

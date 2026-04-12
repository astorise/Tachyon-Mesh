# Tasks: tachyon-ui Clean Slate

- [x] Normalize the change artifacts to the current repository architecture and replace the prose-only `specs.md` with proper OpenSpec deltas.
- [x] Verify that `tachyon-ui` remains a pure Tauri desktop wrapper with no CLI parsing, no manifest-generation path, and a minimal Rust dependency surface around `tachyon-client`.
- [x] Verify that the existing Vite, Tailwind, and GSAP frontend assets remain rooted in the flattened `tachyon-ui` crate layout.
- [x] Validate the desktop pipeline with `npm run build`, `npm run tauri build`, `cargo build`, and `openspec validate --all`.

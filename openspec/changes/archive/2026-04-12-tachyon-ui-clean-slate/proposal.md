# Proposal: tachyon-ui Clean Slate

## Why
`tachyon-ui` has already been refactored into a pure Tauri desktop wrapper, but the new change artifacts still describe an older state of the repository: Tauri v1 vocabulary, destructive rewrites, and UI-side CLI manifest generation. That mismatch keeps OpenSpec blocked and leaves the canonical specs inconsistent with the code that actually builds.

## What Changes
- Normalize the change artifacts to the current repository architecture (`tachyon-ui`, Tauri v2, crate-local `frontendDist`).
- Lock in the clean-slate `tachyon-ui` contract: no CLI parsing, no manifest-generation path, and a minimal Rust dependency surface around `tachyon-client` plus Tauri.
- Preserve the existing Vite/Tailwind/GSAP frontend assets as the supported desktop layout and validate the full Tauri packaging pipeline.

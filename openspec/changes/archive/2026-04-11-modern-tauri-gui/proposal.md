# Proposal: Change 061 - UI Integration (Vite + Tailwind + GSAP)

## Why
The `tachyon-cli` desktop application still ships with a placeholder HTML payload even though the Tauri host runtime is already present. We need a real frontend toolchain so the provided UI can build repeatably inside the existing Tauri v2 crate layout.

## What Changes
1. Bootstrap Vite, Tailwind CSS, TypeScript, and GSAP inside `tachyon-cli`.
2. Replace the placeholder frontend with the provided HTML, CSS, and TypeScript assets without changing the visual design.
3. Expose a minimal Tauri command named `get_engine_status` from the existing Rust crate so the frontend can invoke it.
4. Keep the implementation aligned with the repository’s actual Tauri v2 layout, which uses `tachyon-cli/src/main.rs` and `tachyon-cli/src/lib.rs` instead of a separate `src-tauri` directory.

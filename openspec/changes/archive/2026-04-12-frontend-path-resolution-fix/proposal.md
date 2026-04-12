# Proposal: Frontend Path Resolution Fix

## Context
`tachyon-ui` now uses the Tauri v2 configuration schema, where the frontend bundle path is declared through `build.frontendDist`. The intent of this change is to lock in the correct crate-local asset resolution so future edits do not regress the desktop bundle back to an invalid parent-relative path.

## Objective
Keep the Tauri frontend bundle path pointed at the Vite output directory inside the `tachyon-ui` crate (`dist`), and validate that both the frontend build and the desktop packaging pipeline resolve assets from that location successfully.

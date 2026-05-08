# Title: Critical Core Integration: Apply Pipeline, Cryptographic Sealing, and UI Security

## Problem Statement
A recent audit revealed severe structural disconnects between the UI/MCP layers and the Rust backend:
1. **Fake Configuration:** UI panels (Traffic, AI, RBAC, etc.) perform local WIT validation but do not push manifests to the mesh (`/admin/manifest`). They are functional false positives.
2. **Broken Seal Flow:** Both the UI and the MCP (`tachyon_register_resource`) leave configurations in a "pending CLI re-seal" state, but provide no mechanism to actually seal and apply the overlay.
3. **Critical Security Flaw:** The UI currently stores user credentials in cleartext via browser `localStorage`.
4. **Weak MCP Capabilities:** The MCP lacks the ability to push manifests (`tachyon_apply_manifest`), rendering it read-only.

## Objective
Connect the frontend and MCP layers to the actual Rust mesh engine. 
1. Implement a unified "Seal & Apply" pipeline that cryptographically signs local overlays and POSTs them to the host.
2. Replace `localStorage` with `tauri-plugin-stronghold` for native, encrypted credential management.
3. Empower the MCP with `tachyon_seal_overlay` and `tachyon_apply_manifest` tools so AI agents can actively pilot the mesh.
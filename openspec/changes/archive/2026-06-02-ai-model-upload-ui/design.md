## Context

The backend and bridge layers are complete:
- `model-broker` (system FaaS): chunked `POST /admin/models/init` → `/admin/models/upload/{id}` → `/admin/models/commit/{id}`, then notifies the registry over HTTP.
- `tachyon-client::push_large_model_with_progress(path, cb)` drives the chunked upload and reports a percentage.
- Tauri command `push_large_model(path) -> Result<String,String>` ([main.rs:612](tachyon-ui/src/main.rs#L612)) wraps it and `app.emit("upload_progress", percentage)` on each chunk; it is registered in the invoke handler and listed in `requiresStepUp` in [network.ts:104](tachyon-ui/src/utils/network.ts#L104).

What is missing is purely the front-end entry point. Tauri stack is v2; the only plugin present is `tauri-plugin-stronghold`. There is no file-dialog plugin and no existing file-picker code.

## Goals / Non-Goals

**Goals:**
- Give operators a working upload control in the AI view that drives `push_large_model` with step-up.
- Show live progress via the existing `upload_progress` event and a clear success/error result.
- Make the post-upload registry flow discoverable (model shows in `/v1/models`).

**Non-Goals:**
- Reworking the `aiController` mock "deploy" payload (separate concern).
- Server-side changes to `model-broker` or the upload protocol.
- Building a full model-management table (list/delete) — only upload + a pointer to where the model appears.
- Resumable/abortable uploads beyond what the broker already supports.

## Decisions

**D1 — File selection: `tauri-plugin-dialog` invoked Rust-side via a custom app command.** `push_large_model` needs a real filesystem path; an HTML `<input type=file>` only yields a sandboxed `File`, not a path, under Tauri. **Implementation note (revised at apply time):** this app currently has *no* capabilities scaffolding (`gen/schemas/capabilities.json` is `{}`, no `capabilities/` dir) — every existing command is an app-defined command, which needs no capability grant, and `tauri-plugin-stronghold` is used only from Rust. Calling the dialog plugin's JS `open()` would be a *plugin command* and would force introducing a capabilities file. To stay consistent and minimal, we instead register `tauri-plugin-dialog` and expose a small app command `pick_model_file() -> Option<String>` that uses the plugin's Rust API (correct native-dialog threading) and returns the chosen path. The frontend calls `pick_model_file` like any other app command (no capability, no JS plugin dependency). Alternative — JS `@tauri-apps/plugin-dialog` + a `dialog:allow-open` capability — rejected: it adds capability scaffolding this app otherwise doesn't use. Alternative — the `rfd` crate — rejected: duplicates a maintained plugin and has its own event-loop caveats.

**D2 — Reuse the privileged-command wrapper, no new policy.** The panel calls the existing `network.ts` wrapper, which already routes `push_large_model` through step-up MFA. We do not add a second auth path or call `invoke` directly. This keeps the security posture (signed/step-up) identical to other privileged commands.

**D3 — Progress via a scoped event listener.** The panel registers a Tauri `listen("upload_progress", …)` for the duration of an upload and unlistens on completion/failure. A single in-flight upload is assumed (the broker and command are single-shot); concurrent uploads are out of scope, so the panel disables the control while an upload is running.

**D4 — Component pattern mirrors existing domain panels.** Implement as a custom element `<tachyon-model-upload-panel>` under `components/domains/`, following the established panel structure (render + i18n + testable logic), mounted by `aiOrchestration.ts`. State machine: `idle → selecting → uploading(pct) → success(assetRef) | error(msg)`.

**D5 — Discoverability, not a model list.** On success, show the returned asset ref and a one-line hint that the model is being registered and will appear in the model list (`/v1/models` via `guest-openai`). We do not build a live model table here; we only point to it.

## Risks / Trade-offs

- **[New dependency surface]** `tauri-plugin-dialog` adds a Rust + JS dependency and a capability grant. → It is a first-party, maintained Tauri plugin; the capability is scoped to open-file only.
- **[Path availability across platforms]** the dialog returns a platform path string consumed by the Rust client. → `push_large_model_with_progress` already takes a `&str` path and runs host-side; the dialog returns exactly that.
- **[Gating confusion]** the panel is invisible without `has_ai`. → Document that model upload needs `model-broker` active (`ai-inference`); when the AI view is visible but `model-broker` is absent, the upload call will surface the broker's backend error via the normal error path.
- **[Step-up friction]** every upload triggers MFA. → Intended; uploads are privileged and infrequent.

## Migration Plan

1. Add `tauri-plugin-dialog` (Rust) + `@tauri-apps/plugin-dialog` (JS) and the `dialog:allow-open` capability.
2. Add `TachyonModelUploadPanel.ts` + tests; mount it in `aiOrchestration.ts`.
3. Add i18n strings.
No data migration; rollback is removing the panel mount + plugin.

## Open Questions

- Should the panel also expose the broker's `abort` endpoint for a cancelled selection, or is closing the panel sufficient? (Default: rely on broker GC; no explicit abort button in v1.)

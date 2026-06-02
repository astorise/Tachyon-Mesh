## Why

The full model-upload chain exists end to end — `system-faas-model-broker` (`/admin/models/init|upload|commit`), the `tachyon-client` `push_large_model_with_progress`, the Tauri command `push_large_model` (emitting an `upload_progress` event), and its step-up entry in `network.ts` — but **no UI ever calls it**. The AI Orchestration view only shows a static asset reference and a "deploy" button that `console.log`s a mock payload. An operator therefore has no way to upload a model from Tachyon UI even though the backend is ready.

## What Changes

- **New `<tachyon-model-upload-panel>` component** (`tachyon-ui/src/components/domains/TachyonModelUploadPanel.ts`), mounted in the AI Orchestration view, that lets an operator select a local model file and upload it.
- **File selection** uses a native picker (HTML `<input type=file>` does not yield a real filesystem path under Tauri). This requires a file-open dialog mechanism — see design for the plugin-vs-command decision; treat as a prerequisite.
- **Upload invocation** goes through the existing privileged-command wrapper in `utils/network.ts`; `push_large_model` is already in the `requiresStepUp` set, so step-up MFA is enforced automatically.
- **Progress + result**: the panel subscribes to the `upload_progress` Tauri event and renders a progress bar; on success it surfaces the returned asset ref and a hint that the model will appear in the model list (`GET /v1/models`); on error it shows the translated backend error.
- **Discoverability of the post-upload flow**: the panel notes/links that an uploaded model is registered automatically (`model-broker` commit → `/internal/guest-openai/register` → `guest-openai` registry), so it shows up in `/v1/models` without further action.
- The mock-only "deploy" button behavior in `aiController` is left unchanged by this change (out of scope) except where the upload panel needs to coexist.

## Capabilities

### New Capabilities

- `ai-model-upload-ui`: An operator-facing panel in Tachyon UI to upload a local model file to the cluster via the model broker, with native file selection, step-up-gated invocation, live upload progress, and post-upload registry discoverability.

### Modified Capabilities

- `ai-orchestration`: the `AI Orchestration Panel` requirement is extended to state that the AI view hosts the model-upload panel (visible under the existing `has_ai` gating). No change to the existing payload-validation or accelerator requirements.

## Impact

- **New**: `tachyon-ui/src/components/domains/TachyonModelUploadPanel.ts` (+ test), wired into `tachyon-ui/src/views/aiOrchestration.ts`.
- **Prerequisite**: a native file-open mechanism — either `tauri-plugin-dialog` (+ `@tauri-apps/plugin-dialog`, capability permission in `tauri.conf.json`/capabilities) or a small custom `#[tauri::command]` using a file-dialog crate. No dialog plugin is currently present.
- **`utils/network.ts`**: reuse the existing privileged wrapper; no policy change (`push_large_model` already requires step-up).
- **i18n**: new strings for the panel (`tachyon-ui/src/utils/i18n.ts`).
- **Gating**: panel only renders inside the AI view, shown when `has_ai` is true (active `model-broker`/`buffer`); model upload specifically requires `model-broker` (an `ai-inference` build).

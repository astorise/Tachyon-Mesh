# Changelog

## Unreleased

- Added the node registry and systems catalog surfaces: `control-plane-faas` can now import `kv-partition`, `system-faas-node-registry` persists enrolled nodes, and the UI exposes read-only Nodes and Systems views.
- Honest policy views: five write-only policy panels (resilience, identity-config, rbac, supply-chain, fleet) now display a "Policy form" badge; topology panel gains a View/Edit mode toggle (defaults to View) with session persistence.
- **FIPS + musl Alpine build** (`Dockerfile.fips`): new multi-stage Dockerfile using `rust:alpine` as the FIPS builder to compile `core-host --features fips` with musl libc, producing a static `FROM scratch` image (~32 MB). CI gains a dedicated `fips-tests` job and the Docker publish matrix gains a `-fips` variant. A `feature-matrix-tests` job now validates five distinct feature-flag combinations for `core-host`, each uploading a labelled release artifact.
- **AI inference: ORT → candle-onnx** (`core-host/src/ai_inference/candle_onnx_backend.rs`): replaced Microsoft ORT (native FFI, musl-incompatible) with Hugging Face `candle-onnx` (pure Rust). WASI-NN guest API is preserved — guests use the same `graph_load → init_execution_context → set_input → compute → get_output` flow. Models load from raw ONNX bytes decoded via `prost`. GPU inference is deferred pending upstream candle issue #3491 (CPU-only for now). The `ai-inference` feature is now musl-compatible.
- **Homelab K3S deployment**: provisioned two K3S instances (`tachyon-edge-1` and `tachyon-edge-2`) via WSL/MCP and deployed the latest `core-host` image from GHCR using hardened Kubernetes manifests with Pod Security Standards, NetworkPolicy, and GPU node-selector support.

## [v1.0.0] — General Availability (GA) · 2026-05-15

Tachyon-Mesh v1.0.0 marks the transition to a production-ready, Enterprise-grade FaaS and AI orchestration mesh. This major release signals the stabilisation of our core API and MCP contracts. Following a rigorous multi-stage usability and security audit, it delivers robust supply-chain security, a hardened LLM agent interface, and full WCAG AAA accessibility.

---

### 🛡️ Security & Supply Chain

- **Keyless Signing:** All Linux/macOS release artifacts are cryptographically signed using GitHub OIDC and Sigstore (`cosign sign-blob`). Rekor transparency-log bundles (`.bundle`) are attached to every release.
- **SBOM Generation:** SPDX 2.3 Software Bill of Materials are generated via `cargo sbom` and attached to releases for vulnerability scanning (Trivy, Grype, etc.).
- **Zero-Build Verification:** The `get-tachyon.sh` and `get-tachyon.ps1` installation scripts now enforce SHA-256 checksum validation before extraction, preventing MITM attacks.
- **XSS Immunity:** The entire UI has been rewritten to use native DOM APIs (`el()` / `frag()` helpers). All `innerHTML` interpolation of user data has been eliminated. A strict Content Security Policy is compatible with the new architecture.
- **GitHub Actions Hardening:** `publish-server-binaries` now requests `id-token: write` for keyless OIDC. Artifact names are resolved via `$GITHUB_OUTPUT` (step outputs) rather than `$GITHUB_ENV` for static linter correctness.

---

### 🤖 AI Agents & MCP (Claude Desktop / Cursor)

- **Pre-auth Parameter Validation:** `missing_required_args()` now runs before authentication and network calls, ensuring malformed requests return `-32602 Invalid Params` rather than `-32001 Cluster Unreachable`. Agents can self-correct without a live cluster.
- **Strict Rate Limiting:** All mutator tools are rate-limited before dispatch: `canary_split` at 2/min, `deploy/delete` at 5/min, KV mutators at 30/min. Rate limits fire even when the cluster is unreachable.
- **Error Taxonomy:** Standardised error codes across all 35 MCP tools: `-32602` (invalid params), `-32001` (cluster unreachable), `-32002` (rate limited with `retry_after_ms`), `-32603` (internal error).
- **E2E Test Coverage:** Full behavioural coverage for all lifecycle (`deploy_function`, `list_functions`, `function_logs`, `delete_function`) and KV (`kv_get`, `kv_put`, `kv_delete`) tools. All 12 tests pass without a live cluster.
- **`tools/list` Warnings:** When the manifest schema is not yet fetched, `tools/list` appends a `data.warnings` array instead of returning degraded results silently.
- **Rate-limit State Isolation:** Tests use unique per-process state paths to prevent inter-test contamination.

---

### ♿ UI & Accessibility (WCAG AAA)

- **Focus Restoration (WCAG 2.4.3):** `trapFocus()` now captures `previousFocus` on entry and restores it in the cleanup function. Keyboard users are no longer dropped to `<body>` after closing a modal.
- **Screen Reader Announcements:** `TachyonToastManager` container has `role="status" aria-live="polite" aria-atomic="false"`. All `sealAndApply` outcomes (success, conflict, error) are announced.
- **`<dialog>` Safety Docs:** `trapFocus()` carries a JSDoc warning that it must not be combined with native `<dialog>` elements to prevent duplicate focus trapping.
- **Global Apply Loader:** `aria-live="polite"`, `role="status"`, `.sr-only` descriptive text, and `aria-hidden` on the visual spinner. `aria-busy="true"` is set on `main-content` during apply.
- **Component Decomposition:** `TachyonAppShell` split into `TachyonAppShellNav` and `TachyonAppShellModalRoot`. `TachyonIAM` split into `TachyonAuthStepCredentials`.

---

### ☸️ Kubernetes & Infrastructure

- **Hardened Homelab Manifest:** `manifests/deploy-gpu-homelab-hardened.yaml` enforces Pod Security Standards (Restricted): dedicated `ServiceAccount`, `runAsNonRoot`, `readOnlyRootFilesystem`, `capabilities.drop: [ALL]`, `seccompProfile: RuntimeDefault`, `/tmp emptyDir` scratch volume.
- **Zero-Trust NetworkPolicy:** Default-deny ingress/egress with explicit allowances for port 8080 (MCP/API), Prometheus scraping from the `monitoring` namespace, DNS (UDP+TCP/53), and HTTPS (TCP/443).
- **Dynamic OpenAPI:** All 35 core-host routes are documented and served at `/admin/docs` (Swagger UI) and `/admin/schema/openapi.json`. The integrity.lock JSON Schema is served at `/admin/schema/integrity-lock`.
- **IDE Schema Integration:** VS Code `json.schemas` and YAML modeline configs for `integrity.lock` validation. See `docs/ide-integration.md`.
- **GPU Homelab:** `deploy-gpu-homelab.yaml` supports NVIDIA GPU nodeSelector, `tachyon-model-pvc` (50 Gi), `ServiceMonitor` for Prometheus, and `CUDA_VISIBLE_DEVICES` injection.

---

### 🔧 Developer Experience

- **Setup Scripts:** `scripts/setup.sh` and `scripts/setup.ps1` bootstrap the full development environment (WASM targets, binaries, guest artifacts, npm install) with `--skip-guests` and `--skip-ui` flags.
- **Zero-Build Installers:** `get-tachyon.sh` and `get-tachyon.ps1` download pre-built binaries, verify SHA-256, and print an MCP config banner.
- **TROUBLESHOOTING.md:** 15 documented failure modes covering build, runtime, UI, MCP, and Kubernetes/GPU domains.
- **`docs/ide-integration.md`:** VS Code, JetBrains, Neovim integration guides and offline schema snapshot procedure.

---

## [v0.x] — Pre-release development

All earlier development was pre-release. No public changelog is maintained for the `0.x` series.

## 1. Public Route Contract

- [x] 1.1 Move `guest-openai` dispatch constants and route tests to `/ai/v1/models` and `/ai/v1/chat/completions`
- [x] 1.2 Update guest example manifests, integrity fixtures, sealing scripts, and gateway route expectations
- [x] 1.3 Add validation coverage proving the former `/v1/*` routes are absent

## 2. Consumers and Documentation

- [x] 2.1 Update Tachyon client calls, MCP descriptions, UI copy, translations, comments, and tests
- [x] 2.2 Update active OpenSpec requirements and developer documentation to the `/ai/v1/*` contract
- [x] 2.3 Configure Continue with `https://tachyon-mesh.wsl/ai/v1`

## 3. Verification

- [x] 3.1 Run guest-openai, gateway, host validation, client, MCP, and UI tests affected by the route move
- [x] 3.2 Rebuild the `guest-openai` WASM and regenerate or reseal tracked integrity artifacts
- [x] 3.3 Confirm no active source reference still treats `/v1/*` or `ai.tachyon-mesh.wsl` as canonical

## 4. HomeLab Migration

- [x] 4.1 Reseal the live manifest with `/ai/v1/*` while preserving the internal registration route and Qwen model binding
- [x] 4.2 Smoke-test model listing and chat completions through `https://tachyon-mesh.wsl/ai/v1`
- [x] 4.3 Verify `/v1/*` returns 404 and remove the `ai.tachyon-mesh.wsl` HomeLab route

# Design: Audit Closure

## Context

The audit identified four closure items: WIT contract drift in the UI backend, an unauthenticated MCP stdio server, unsafe rendering patterns around runtime strings, and raw backend errors shown directly to operators.

## Decisions

- Add `wit-bindgen` to the Tachyon-UI backend and generate bindings from the workspace WIT folder during compilation.
- Replace the old UI backend DTO validation with JSON validation aligned to the WIT shapes used by current panels, avoiding stale duplicated Rust structs.
- Require `--token` or `TACHYON_MCP_PAT` before the MCP server accepts input. If `TACHYON_MCP_URL` is present, validate the PAT against the node through `tachyon-client`.
- Rate-limit MCP write tools to five writes per minute in process.
- Replace dynamic runtime `innerHTML` paths with DOM construction and `textContent`.
- Translate known backend errors in `resilientInvoke` so component-level catches and toasts receive localized messages.

## Risks

- Some existing UI panel payloads still use compact form models rather than the full WIT record tree. Validation therefore accepts both current panel payloads and WIT-shaped objects while the generated WIT module keeps contract drift visible at compile time.
- MCP PAT validation requires a host URL; when `TACHYON_MCP_URL` is absent, startup still requires a PAT but cannot perform remote validation.

# Design: mcp-mutator-hardening

## Task 1 — Granular Mutator Rate Limits

`rate_limit_spec()` in `tachyon-mcp/src/main.rs` is extended with a complete tool-name→limit mapping:

| Tier | Tools | Limit |
|---|---|---|
| Critical canary | `tachyon_canary_split` | 2 / 60 s |
| Manifest sealing | `tachyon_apply_manifest`, `tachyon_seal_overlay` | 1 / 60 s |
| Deploy / delete | `tachyon_deploy_function`, `tachyon_delete_function` | 5 / 60 s |
| Resource registration | `tachyon_register_resource` | 10 / 60 s |
| KV mutators + logs | `tachyon_kv_put`, `tachyon_kv_delete`, `tachyon_function_logs`, `tachyon_get_metrics`, `tachyon_tail_logs` | 30 / 60 s |
| Read-only | all other named tools | 100 / 60 s |

Exceeding any budget returns the standardised `-32002` `JsonRpcError::rate_limited(retry_after_ms)` response.

## Task 2 & 3 — Schema Enrichment and LLM Guidance

**`tachyon_deploy_function`**: Description updated to explicitly state that `artifact_path` must be an absolute path on the host machine running the MCP server. Example path added to the property description.

**`tachyon_kv_put`**: Description updated to mandate JSON-stringified values; namespace and key descriptions added; value property description includes an example.

**`tachyon_kv_delete`**: Description updated with a permanence warning. Namespace/key descriptions added.

**`tachyon_canary_split`**: Description updated to explain incremental rollout semantics (`weight_pct=0` = rollback, 1-100 = partial / full promotion), with a suggested progression example (`10→25→50→100`).

## Task 4 — Dead Code Removal

The legacy `error_response(id, code, message)` function (which produced an unstructured error object) has been deleted. Its single remaining call site (line 356 — the top-level `Err` handler in the main stdio loop) is replaced with `json_rpc_error_response(None, &JsonRpcError::from_anyhow(&error))`, which uses the standardised structured error object consistent with all other error paths in the file.

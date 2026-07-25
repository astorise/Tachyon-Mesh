# RAG vector search demo

This guide wires the end-to-end RAG example from `examples/guest-rag-vector`:

1. ingest documents,
2. call the OpenAI-compatible embeddings route when available,
3. upsert vectors through the `tachyon:mesh/vector` WIT import,
4. search the ANN index,
5. call `/ai/v1/chat/completions` with retrieved context when available,
6. expose the same query flow to agents through `tachyon_vector_search`.

The example has local deterministic fallbacks for embeddings and completion, so
it can be smoke-tested before a real model is uploaded. When `/ai/v1/embeddings`
and `/ai/v1/chat/completions` are backed by `guest-openai`, the same guest uses
those routes through Tachyon's internal mesh dispatch path.

## Build the guest

```powershell
cargo build -p guest-rag-vector --target wasm32-wasip2 --release
```

The artifact is emitted at:

```text
target/wasm32-wasip2/release/guest_rag_vector.wasm
```

## Import the route

`examples/guest-rag-vector/manifest.json` declares `/api/guest-rag-vector` with:

- `vector` scope for `tenant-kb` and `demo-*`,
- `http` scope for `/ai/v1/embeddings` and `/ai/v1/chat/completions`,
- dependencies on the OpenAI-compatible routes used for in-process dispatch.

Package the WASM and manifest with the same archive shape accepted by
`tachyon_import_package`, then import it:

```json
{
  "package_path": "C:\\path\\to\\guest-rag-vector.tar.gz"
}
```

If you prefer direct deployment while iterating, upload the WASM with
`tachyon_deploy_function`, then apply the scopes from the manifest with
`tachyon_patch_route`.

## Smoke-test HTTP RAG

```powershell
$body = @{
  query = "How does the MCP vector tool reach RAG context?"
  index = "tenant-kb"
  topK = 3
} | ConvertTo-Json

Invoke-RestMethod `
  -Method Post `
  -Uri "$env:TACHYON_MCP_URL/api/guest-rag-vector" `
  -Headers @{ Authorization = "Bearer $env:TACHYON_MCP_PAT" } `
  -ContentType "application/json" `
  -Body $body
```

Expected response fields:

- `answer`: model answer or fallback answer grounded in the best match,
- `matches`: nearest documents with scores and payload text,
- `effectiveIndex`: temporary per-request, dimension/source-specific index used internally,
- `embeddingSource`: `openai-compatible:<model>` or local fallback,
- `completionSource`: `openai-compatible:<model>` or local fallback.

To ingest your own demo corpus for the request:

```json
{
  "query": "What protects outbound calls?",
  "index": "tenant-kb",
  "topK": 2,
  "documents": [
    { "id": "scopes", "text": "Tachyon validates WIT imports against per-route scopes." },
    { "id": "dispatch", "text": "Internal mesh routes can dispatch in-process on the same node." }
  ]
}
```

## Agent access through MCP

`tachyon-mcp` now advertises the read-only tool:

```json
{
  "name": "tachyon_vector_search",
  "arguments": {
    "query": "What is the vector WIT interface used for?",
    "index": "tenant-kb",
    "top_k": 3
  }
}
```

Optional arguments:

- `route_path`: override the default `/api/guest-rag-vector`; can also be set
  with `TACHYON_MCP_VECTOR_SEARCH_PATH`,
- `embedding_model`: embedding model alias passed to the RAG route,
- `chat_model`: chat model alias passed to the RAG route,
- `documents`: temporary documents to ingest before the search.

The MCP server applies the normal connection/auth flow (`TACHYON_MCP_URL` and
`TACHYON_MCP_PAT`), injects an internal request identifier so temporary vector
documents are isolated per call, and rate-limits the tool as a high-throughput
read operation.

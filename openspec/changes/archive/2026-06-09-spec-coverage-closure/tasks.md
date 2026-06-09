## 1. system-faas-prom exposition (new capability)

- [x] 1.1 Record the privileged-reader dependency: the component calls
      `telemetry_reader::get-metrics()` from the `system-faas-guest` world.
- [x] 1.2 Specify the nine `tachyon_*` series, their `# TYPE` annotations
      (counter/gauge), and the snapshot field each renders.
- [x] 1.3 Specify the `200` Prometheus-text response returned for any request.

## 2. tachyon_upload_model MCP tool (mcp-server)

- [x] 2.1 Specify the tool name, the required `path` argument, and delegation to
      `tachyon_client::push_large_model`.
- [x] 2.2 Specify the registered schema (`required: ["path"]`), the pre-dispatch
      missing-param rejection, and the tight rate-limit budget.

## 3. Distributed limiter sync surface (distributed-crdt-rate-limiter)

- [x] 3.1 Specify the `POST /check`, `POST /merge`, and `GET /state` endpoints
      with their request/response bodies and status codes.
- [x] 3.2 Specify the `{key}:{window}` time-window keying derived from
      `DIST_LIMIT_WINDOW_SECONDS`.
- [x] 3.3 Specify the per-(key,node) maxima merge that makes the G-counter
      convergent across nodes.

## 4. TEE backend contract (confidential-computing-tee)

- [x] 4.1 Specify backend selection from the sealed `tee_backend` and the `503`
      fail-closed path when none is configured.
- [x] 4.2 Specify the `LocalEnclave` (no hardware isolation) vs `Enarx`
      (`enarx` feature, Keep endpoint) distinction.
- [x] 4.3 Specify the Enarx Keep invocation protocol and the
      `x-tachyon-runtime` / `x-tachyon-tee-backend` response annotations.

## 5. Metering exporter ownership + durable outbox (tracing-metering)

- [x] 5.1 Realign the metering-aggregation requirement to the host exporter
      (in-memory batching + size / `TELEMETRY_EXPORT_FLUSH_INTERVAL` flush).
- [x] 5.2 Specify the durable `metering_outbox` staging: persist before export,
      delete on success, retain on failure.
- [x] 5.3 Keep the requirements within what the code guarantees — no claim of
      automatic re-export of retained entries.

## 6. Verification

- [x] 6.1 Confirm every new requirement matches current code behavior, not an
      aspirational target.
- [x] 6.2 Confirm no code, dependency, or WIT change is implied by this change.
- [x] 6.3 Confirm the `tracing-metering` MODIFIED requirement keeps the exact
      original title so archiving replaces it cleanly.

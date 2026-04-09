# Tasks: Change 036 Implementation

**Agent Instruction:** Read the proposal.md and specs.md. Implement the Smart Proxy and the CDC poller System FaaS. Do not use nested code blocks in your outputs.

## [TASK-1] Implement the S3 Smart Proxy FaaS
- [ ] Create a WASM component named system-faas-s3-proxy.
- [ ] Read the REAL_S3_BUCKET and TARGET_ROUTE from environment variables.
- [ ] In the HTTP handler, accept incoming PUT requests containing file bodies.
- [ ] Stream the body to the external REAL_S3_BUCKET URL using the outbound HTTP capability. Ensure proper authorization headers are appended.
- [ ] If the external storage returns a success status, trigger an asynchronous POST request to the TARGET_ROUTE via the internal mesh capability, passing the file metadata as a JSON payload.
- [ ] Return a success response to the original caller.

## [TASK-2] Implement the CDC Outbox Poller FaaS
- [ ] Create a WASM component named system-faas-cdc.
- [ ] Read the DB_URL, OUTBOX_TABLE, and TARGET_ROUTE from the environment.
- [ ] Establish a connection to the database (using a WASM-compatible Postgres/MySQL driver or via a new Host capability if raw TCP sockets are insufficient).
- [ ] Implement a loop that polls the OUTBOX_TABLE. Select rows safely using a row-level lock (e.g., FOR UPDATE SKIP LOCKED).
- [ ] Iterate through the fetched rows. For each row, POST its payload to the TARGET_ROUTE.
- [ ] If the target returns a 200 OK status, execute a DELETE query for that specific row ID to acknowledge processing.

## Validation Step
- [ ] Start a local PostgreSQL instance and create an events_outbox table. Insert a dummy row.
- [ ] Deploy the system-faas-cdc targeting a mock business FaaS.
- [ ] Verify the poller reads the row, triggers the mock FaaS, and deletes the row from PostgreSQL.
- [ ] Deploy the system-faas-s3-proxy. Send a file via curl to the proxy.
- [ ] Verify the file is forwarded to your test S3 bucket, and that the internal Mesh event is subsequently fired.

# Proposal: Change 035 - Event-Driven Triggers (System Connectors)

## Context
A true Serverless platform must react to asynchronous events (Message Queues, Pub/Sub) without requiring the business logic (User FaaS) to act as a long-running subscriber. If a User FaaS polls a queue, it breaks the "Scale-to-Zero" promise and creates concurrency bottlenecks. To match AWS Lambda's event-driven architecture while keeping the Rust host pure and zero-overhead, we must use System FaaS as dedicated "Event Connectors".

## Objective
1. Formalize the "Event Connector" pattern: a long-running System FaaS that subscribes to an external broker (e.g., SQS, RabbitMQ, Kafka) and dispatches messages to ephemeral User FaaS via internal Mesh IPC.
2. Update the `integrity.lock` schema to explicitly declare these event triggers, ensuring the Host can validate the internal dependency graph (Change 025/027).
3. Build a reference `system-faas-sqs` (or generic webhook/poller) that demonstrates polling, triggering a target, and handling ACK/NACK based on the HTTP status returned by the User FaaS.

## Scope
- Update `tachyon-cli` to introduce an `events` or `triggers` block mapping an external source to an internal target route.
- Implement a `system-faas-sqs` component in Rust (compiled to WASM) that uses standard outbound HTTP capabilities to long-poll AWS SQS.
- Implement the dispatch logic: for each message, make a `POST http://mesh/<target>` call. If the target returns 200 OK, delete the message from the queue (ACK). If it fails or times out, leave it in the queue for a retry (NACK).

## Success Metrics
- A User FaaS scales down to 0 RAM consumption when the queue is empty.
- When 10 messages arrive in the queue, the System FaaS pulls them and fires 10 concurrent IPC requests to the Mesh. The Rust Host automatically allocates 10 ephemeral User FaaS instances to process them in parallel.
- A crash in the User FaaS results in an HTTP 500, prompting the System FaaS to NOT delete the message, allowing standard Dead Letter Queue (DLQ) mechanics to take over.
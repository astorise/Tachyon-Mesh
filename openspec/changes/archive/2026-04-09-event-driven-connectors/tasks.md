# Tasks: Change 035 Implementation

**Agent Instruction:** Read the proposal.md and specs.md. Implement the SQS event connector System FaaS. Do not use nested code blocks in your outputs.

## [TASK-1] Update CLI Schema
- [x] In tachyon-cli, ensure the Target configuration can accept standard environment variables that will be used to configure the triggers (e.g., QUEUE_URL and TARGET_ROUTE).
- [x] Ensure the dependency graph validator allows the system FaaS to declare an internal Mesh dependency on the user FaaS it intends to trigger.

## [TASK-2] Create the System FaaS Connector (SQS Poller)
- [x] Create a new WASM component project named system-faas-sqs.
- [x] Read the QUEUE_URL and TARGET_ROUTE from the environment variables (std::env::var).
- [x] Create an infinite loop. Inside the loop, use the WASI HTTP Outbound capability (or reqwest if compiled with WASI Preview 2 HTTP support) to call the SQS ReceiveMessage API endpoint.
- [x] Set the wait time to 20 seconds to enable long-polling and reduce CPU cycles.

## [TASK-3] Implement the Dispatch and ACK Logic
- [x] Parse the JSON response from the broker to extract the messages and their ReceiptHandles.
- [x] For each message, make a synchronous-looking (but actually async via Wasmtime) HTTP POST request to the TARGET_ROUTE via the internal mesh capability.
- [x] Check the response status code of the internal call.
- [x] If the status is 200 OK, make another HTTP call to the broker's DeleteMessage API using the ReceiptHandle to acknowledge the message.
- [x] If the status is an error, simply continue to the next message (the broker will automatically retry it later).

## Validation Step
- [x] Start a local mock SQS server (like ElasticMQ) or use an AWS test queue.
- [x] Deploy the system-faas-sqs and a simple dummy user FaaS that prints its input and returns 200 OK.
- [x] Send 5 messages to the queue.
- [x] Verify via the host logs that the System FaaS pulls the messages, routes them internally to the user FaaS, and deletes them from the queue upon success.
- [x] Modify the user FaaS to panic or return 500. Send a message to the queue. Verify the System FaaS does not delete it, and it reappears in the queue after the visibility timeout.

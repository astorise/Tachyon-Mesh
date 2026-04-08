# Tasks: Change 035 Implementation

**Agent Instruction:** Read the proposal.md and specs.md. Implement the SQS event connector System FaaS. Do not use nested code blocks in your outputs.

## [TASK-1] Update CLI Schema
1. In tachyon-cli, ensure the Target configuration can accept standard environment variables that will be used to configure the triggers (e.g., QUEUE_URL and TARGET_ROUTE).
2. Ensure the dependency graph validator allows the system FaaS to declare an internal Mesh dependency on the user FaaS it intends to trigger.

## [TASK-2] Create the System FaaS Connector (SQS Poller)
1. Create a new WASM component project named system-faas-sqs.
2. Read the QUEUE_URL and TARGET_ROUTE from the environment variables (std::env::var).
3. Create an infinite loop. Inside the loop, use the WASI HTTP Outbound capability (or reqwest if compiled with WASI Preview 2 HTTP support) to call the SQS ReceiveMessage API endpoint.
4. Set the wait time to 20 seconds to enable long-polling and reduce CPU cycles.

## [TASK-3] Implement the Dispatch and ACK Logic
1. Parse the JSON response from the broker to extract the messages and their ReceiptHandles.
2. For each message, make a synchronous-looking (but actually async via Wasmtime) HTTP POST request to the TARGET_ROUTE via the internal mesh capability.
3. Check the response status code of the internal call.
4. If the status is 200 OK, make another HTTP call to the broker's DeleteMessage API using the ReceiptHandle to acknowledge the message.
5. If the status is an error, simply continue to the next message (the broker will automatically retry it later).

## Validation Step
1. Start a local mock SQS server (like ElasticMQ) or use an AWS test queue.
2. Deploy the system-faas-sqs and a simple dummy user FaaS that prints its input and returns 200 OK.
3. Send 5 messages to the queue.
4. Verify via the host logs that the System FaaS pulls the messages, routes them internally to the user FaaS, and deletes them from the queue upon success.
5. Modify the user FaaS to panic or return 500. Send a message to the queue. Verify the System FaaS does not delete it, and it reappears in the queue after the visibility timeout.
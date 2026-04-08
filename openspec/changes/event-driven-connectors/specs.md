# Specifications: Event-Driven Architecture

## 1. Schema Update v10 (integrity.lock)
We must declare the relationship between the System FaaS (Connector) and the User FaaS (Processor).

    {
        "targets": [
            {
                "name": "sqs-connector",
                "module": "system-faas-sqs.wasm",
                "role": "system",
                "env": {
                    "QUEUE_URL": "https://sqs.eu-west-1.amazonaws.com/123/my-queue",
                    "TARGET_ROUTE": "http://mesh/process-order"
                },
                "requires_credentials": ["aws-sqs-read", "mesh-invoke"]
            },
            {
                "name": "process-order",
                "module": "guest-order-worker.wasm",
                "role": "user"
            }
        ]
    }

## 2. Connector Execution Model (System FaaS)
The Connector is compiled as a standard WASI component (Reactor or infinite loop Command).
- It uses the Outbound HTTP capability to send a Long-Polling request to the broker (e.g., SQS ReceiveMessage with WaitTimeSeconds=20).
- Because Wasmtime is strictly asynchronous, this long-poll suspends the WASM module and yields to the Tokio executor. It consumes zero CPU while waiting for messages.

## 3. The ACK / NACK Contract
The System FaaS enforces reliability without the core-host knowing anything about Kafka or SQS:
- Received Message -> Send `POST http://mesh/process-order` with the message payload.
- If HTTP Status == 200..299: The System FaaS calls the broker's DeleteMessage endpoint (ACK).
- If HTTP Status >= 400 or Network Timeout: The System FaaS logs the error and does NOT delete the message. The broker's native visibility timeout will make the message reappear later (NACK).
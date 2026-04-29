# Design: Enrollment State Machine

## 1. Boot Sequence (`core-host/src/main.rs`)
At startup, the host checks for the presence of a valid cluster certificate in its secure storage.
- **If missing:** The host enters "Bootstrap Mode". It restricts all incoming network traffic and exclusively loads `system-faas-enrollment`.
- **If present:** It proceeds to normal operation.

## 2. The Enrollment FaaS (`systems/system-faas-enrollment`)
- **Key Generation:** Calls host WASI crypto to generate a local Ed25519 keypair.
- **PIN Generation:** Generates a 6-character alphanumeric PIN. Outputs this to the console/logs: `[ENROLLMENT] Waiting for approval. Enter PIN in Tachyon-UI: A7X-92B`.
- **Polling/Streaming:** Connects to `https://<cluster-bootstrap-url>/api/enroll` and holds the connection open, sending a heartbeat every 30 seconds.

## 3. UI and Masterless Resolution
- Tachyon-UI sends the command `POST /api/admin/approve-node { pin: "A7X-92B" }` to any active node.
- The active node looks up its pending connections (or gossips the approval to the node holding the connection).
- The certificate is generated, signed by the cluster CA, and streamed back to the waiting FaaS.

## 4. Handoff
Once the FaaS receives the certificate:
1. It writes the cert to the secure volume.
2. It sends an IPC signal to `core-host`: `ENROLLMENT_COMPLETE`.
3. The host tears down the `system-faas-enrollment` instance and initiates the standard hot-reload bootstrap.
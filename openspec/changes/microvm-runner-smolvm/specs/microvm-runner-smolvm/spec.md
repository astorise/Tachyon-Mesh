## ADDED Requirements

### Requirement: MicroVM runtime manifest support
Function manifests MUST be able to declare a MicroVM runtime with an image path and resource limits.

#### Scenario: Function declares a MicroVM image
- **GIVEN** a deployment manifest declares `type: microvm`
- **WHEN** the host parses the function configuration
- **THEN** it accepts the MicroVM runtime fields
- **AND** it validates the image, vCPU, and memory settings before dispatch

### Requirement: SmolVM runner dispatch
The host MUST route MicroVM functions to a dedicated `system-faas-microvm-runner` instead of the Wasm execution pool.

#### Scenario: Request targets a MicroVM function
- **GIVEN** a request is routed to a function configured with the MicroVM runtime
- **WHEN** the dispatcher resolves the function runtime
- **THEN** it delegates the request payload to `system-faas-microvm-runner`
- **AND** it preserves the same external request and response contract as Wasm functions

### Requirement: Guest IPC proxy
The MicroVM runner MUST proxy Tachyon payloads into the guest and return stdout, stderr, status, and response data to the caller.

#### Scenario: Guest agent executes native code
- **GIVEN** a MicroVM has booted with a guest agent listening over vsock or serial IPC
- **WHEN** the runner forwards a Tachyon invocation payload
- **THEN** the guest agent executes the requested native workload
- **AND** the runner returns the guest result or a structured execution error

### Requirement: MicroVM lifecycle controls
The MicroVM runner MUST support cold boot, warm reuse, and snapshot restore according to runtime policy.

#### Scenario: Inactive MicroVM is restored
- **GIVEN** a MicroVM function has an available snapshot
- **WHEN** a new request arrives after the warm instance was hibernated
- **THEN** the runner restores the snapshot when policy allows
- **AND** it enforces the declared CPU and memory limits before resuming execution

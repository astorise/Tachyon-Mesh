## ADDED Requirements

### Requirement: Outbound device enrollment FaaS
Nodes without valid cluster credentials MUST enter bootstrap mode and run only the enrollment System FaaS until enrollment completes.

#### Scenario: Clean node starts without credentials
- **GIVEN** a node starts without a valid cluster certificate or integrity lock
- **WHEN** the host finishes its credential check
- **THEN** it enters bootstrap mode
- **AND** it loads `system-faas-enrollment` as the only executable workload
- **AND** inbound management and mesh traffic remain disabled until enrollment succeeds

### Requirement: PIN-based outbound approval
The enrollment FaaS MUST generate a short-lived PIN, establish an outbound tunnel to a bootstrap endpoint, and use that tunnel to receive signed credentials.

#### Scenario: Administrator approves a pending node
- **GIVEN** the enrollment FaaS has opened an outbound enrollment tunnel and displayed a PIN
- **WHEN** an administrator submits the PIN to an active node
- **THEN** the active node signs the pending node public key
- **AND** the certificate is delivered back over the existing outbound tunnel
- **AND** the enrolling node stores the certificate and exits bootstrap mode

### Requirement: Enrollment handoff
The host MUST transition from bootstrap mode to normal mesh operation after the enrollment FaaS persists credentials.

#### Scenario: Enrollment completes successfully
- **GIVEN** the enrollment FaaS has stored valid cluster credentials
- **WHEN** it signals `ENROLLMENT_COMPLETE` to the host
- **THEN** the host stops the enrollment FaaS
- **AND** it loads the normal mesh overlay and configured workloads
- **AND** subsequent restarts skip bootstrap mode while credentials remain valid

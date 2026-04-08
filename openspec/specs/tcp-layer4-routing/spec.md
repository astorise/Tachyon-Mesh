# tcp-layer4-routing Specification

## Purpose
TBD - created by archiving change tcp-layer4-routing. Update Purpose after archive.
## Requirements
### Requirement: Hosts can bind TCP ports directly to targets
The host manifest SHALL allow operators to map Layer 4 TCP ports to target names so the runtime can dispatch raw TCP streams without HTTP parsing.

#### Scenario: A TCP port is mapped to a target
- **WHEN** the host loads a `layer4` TCP binding from the manifest
- **THEN** it starts a listener for that port and associates accepted sockets with the configured target

### Requirement: TCP targets receive full-duplex WASI streams
The runtime SHALL wire accepted TCP sockets to the target instance through asynchronous full-duplex stdin and stdout pipes so the guest can process binary protocols directly.

#### Scenario: A TCP connection is dispatched to a target
- **WHEN** the host accepts a TCP connection for a Layer 4 target
- **THEN** it connects the socket reader to the guest stdin pipe
- **AND** connects the guest stdout pipe back to the socket writer
- **AND** aborts the paired copy tasks cleanly when either side disconnects

### Requirement: Long-lived TCP instances terminate cleanly on disconnect
The runtime SHALL keep Layer 4 TCP target instances alive for the duration of the connection and release them when the remote client disconnects or the guest exits.

#### Scenario: The remote client closes the TCP session
- **WHEN** the remote client disconnects from a Layer 4 target
- **THEN** the host closes the guest stdin stream
- **AND** allows the guest to exit naturally
- **AND** returns the instance resources to the runtime cleanup path


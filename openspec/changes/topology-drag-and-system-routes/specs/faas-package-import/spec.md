## MODIFIED Requirements

### Requirement: Guest examples manifest activates 9 routes
The `examples/guest-examples/manifest.json` shipped in the `guest-examples.tar.gz`
CI artifact SHALL declare routes for all practical (non-test, HTTP/WS/gRPC) guest
WASMs present in the archive: `guest-ai`, `guest-call-legacy`, `guest-example`,
`guest-grpc` (as `/grpc/hello`), `guest-log-storm`, `guest-loop`,
`guest-voip-gate`, `guest-volume`, and `guest-websocket-echo`.
`guest-flaky`, `guest-malicious`, `guest-tcp-echo`, and `guest-udp-echo` SHALL
be excluded.

#### Scenario: Import activates 9 routes
- **WHEN** an operator imports the `guest-examples.tar.gz` artifact
- **THEN** 9 routes are added to the live manifest (routes_added = 9)
- **THEN** `guest-flaky`, `guest-malicious`, `guest-tcp-echo`, `guest-udp-echo` are NOT activated

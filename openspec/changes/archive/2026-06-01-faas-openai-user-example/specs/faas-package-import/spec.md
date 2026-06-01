## MODIFIED Requirements

### Requirement: Guest examples manifest shipped in CI artifact
The `examples/guest-examples/manifest.json` shipped in the `guest-examples.tar.gz` CI artifact SHALL declare routes for all practical (non-test, HTTP/WS/gRPC) guest WASMs present in the archive: `guest-ai`, `guest-call-legacy`, `guest-example`,
`guest-grpc` (as `/grpc/hello`), `guest-log-storm`, `guest-loop`,
`guest-voip-gate`, `guest-volume`, and `guest-websocket-echo`. It SHALL ALSO
declare the OpenAI-compatible example routes backed by the `guest-openai`
module: `/v1/models`, `/v1/chat/completions`, and
`/internal/guest-openai/register`. `guest-flaky`, `guest-malicious`,
`guest-tcp-echo`, and `guest-udp-echo` SHALL be excluded.

#### Scenario: Import activates the guest and OpenAI example routes
- **WHEN** an operator imports the `guest-examples.tar.gz` artifact
- **THEN** 12 routes are added to the live manifest (routes_added = 12): the 9 practical guest routes plus the 3 `guest-openai` routes
- **THEN** `guest-flaky`, `guest-malicious`, `guest-tcp-echo`, `guest-udp-echo` are NOT activated

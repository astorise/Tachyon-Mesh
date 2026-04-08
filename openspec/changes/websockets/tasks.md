# Tasks: Change 032 Implementation

**Agent Instruction:** Read `proposal.md` and the delta spec under `specs/`. Implement the WebSocket capability securely behind a feature flag.

- [ ] Add the WebSocket feature flag, WIT interface, and route configuration needed to opt targets into WebSocket upgrades.
- [ ] Upgrade opted-in requests in the host, expose the socket through Wasmtime resources, and drive the guest connection asynchronously.
- [ ] Create a guest WebSocket echo component that handles frames until the client closes the connection.
- [ ] Validate the end-to-end upgrade, echo behavior, and guest shutdown path with a real WebSocket client.

# Tasks: Change 053 Implementation

**Agent Instruction:** Implement the Layer 4 TCP proxy. Rely on Tokio async I/O copying to keep byte streaming non-blocking.

- [ ] Parse Layer 4 TCP bindings and start a listener task for each configured port.
- [ ] Wire accepted TCP streams into guest stdin and stdout with full-duplex async copying and explicit disconnect cleanup.
- [ ] Keep TCP-bound instances alive for the lifetime of the connection and return them to cleanup when the guest exits or the client disconnects.
- [ ] Validate the TCP echo flow and clean instance teardown with a simple Layer 4 test service.

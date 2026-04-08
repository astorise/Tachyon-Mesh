# Proposal: Change 055 - Dynamic L4 Bridging & Media Proxying

## Context
High-frequency UDP protocols like RTP (VoIP) or WebRTC (Video) send packets every 10-20ms. In Change 054, each packet triggers a WASM execution, which creates unsustainable overhead for media-heavy applications. To support services like a PBX (Asterisk-style) or a Video SFU, Tachyon needs a way to "bridge" UDP streams directly at the Host level, bypassing the WASM sandbox for the data plane while keeping control logic in WASM.

## Objective
1. Create a `system-faas-bridge` that acts as a privileged controller for the Host's network stack.
2. Implement an API allowing a FaaS to request dynamic UDP port allocation and "Zero-WASM" relaying (bridging).
3. Ensure high-performance packet forwarding in the Rust Host using specialized Tokio tasks, with zero copying to the WASM guest during the active session.

## Scope
- Define the `tachyon:network/bridge` WIT interface.
- Implement the `DynamicBridgeManager` in the `core-host`.
- Create the `system-faas-bridge` singleton to manage session lifecycles.

## Success Metrics
- A SIP FaaS can request a media bridge for two clients.
- The Host relays 50 packets per second (RTP) between two UDP ports with <1ms jitter and 0% WASM execution overhead during the call.
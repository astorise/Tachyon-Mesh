# Proposal: Change 056 - Global L4 Bridge Steering

## Context
As Tachyon scales to support media proxying (Change 055), local network interfaces or CPU cores handling L4 Tokio bridging tasks can saturate. Overflowing raw packet streams across the internal mesh introduces unacceptable latency and doubles bandwidth usage (Trombone effect). Overflow decisions must be made at the Control Plane level, during the initial port allocation phase.

## Objective
1. Extend the `system-faas-gossip` to broadcast L4 network load (active bridges, bandwidth usage).
2. Upgrade `system-faas-bridge` to become Mesh-Aware. When a local bridge request occurs under high load, the system transparently proxies the allocation request to a peer node via internal mTLS.
3. Ensure the allocated bridge configuration returns the true Public IP/External IP of the node actually hosting the ports.

## Scope
- Update Telemetry to monitor Tokio active L4 tasks and byte rates.
- Modify the `system-faas-bridge` routing logic to use the Mesh P2C (Power of Two Choices) algorithm.
- Add `public_ip` to the Tachyon host configuration.

## Success Metrics
- A node with 1000 active VoIP bridges receives a request for a 1001st bridge.
- The local `system-faas-bridge` securely forwards the request to an idle peer node.
- The returned SIP SDP payload contains the idle peer's IP address, completely offloading the data plane from the saturated node.
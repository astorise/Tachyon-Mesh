# Proposal: Change 054 - Layer 4 UDP Datagram Routing

## Context
Following Change 053 (TCP routing), Tachyon Mesh must support UDP to become a fully-fledged PaaS. Unlike TCP, UDP is connectionless and message-oriented. We cannot pipe UDP traffic into standard I/O streams because the WASM instance needs to know packet boundaries and the origin IP address to respond correctly (e.g., a DNS server replying to a specific client).

## Objective
1. Extend the `layer4` configuration in `integrity.lock` to support UDP ports.
2. Define a specific WIT interface (`tachyon:network/udp`) allowing the FaaS to receive a datagram, its source IP, and return an array of outbound datagrams.
3. Treat UDP packets as discrete, ultra-fast events: the `core-host` receives a packet, pulls a hot WASM instance from the pool, calls the export, sends the response packets, and immediately returns the instance to the pool.

## Scope
- Update `integrity.lock` schema for UDP bindings.
- Create the `tachyon-udp.wit` interface.
- Implement the `tokio::net::UdpSocket` listener in the `core-host`.

## Success Metrics
- A WASM FaaS acting as a custom DNS server is bound to UDP port 53.
- When a user runs `dig @localhost tachyon.local`, the Rust host passes the UDP payload to the FaaS, which returns the correct A record payload in under 2 milliseconds.
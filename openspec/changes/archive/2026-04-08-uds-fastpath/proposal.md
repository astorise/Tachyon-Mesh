# Proposal: Change 037 - Intra-Node Fast-Path (UDS & Local Discovery)

## Context
In a distributed environment like Kubernetes, multiple Tachyon core-host instances may run on the same physical node. Standardizing communication via TCP/mTLS for these local peers is inefficient due to the overhead of the Linux network stack. To achieve true "Zero-Overhead" performance, the Mesh must automatically detect when a peer is local and switch the transport layer to Unix Domain Sockets (UDS), bypassing the entire TCP/IP stack.

## Objective
1. Implement a Node-Local Discovery mechanism where each Tachyon host registers its presence on the local filesystem.
2. Introduce a "Fast-Path" transport in the Mesh Router: before initiating a TCP connection to a peer, the host checks for a local UDS socket.
3. Fallback gracefully to mTLS/TCP if the peer is remote or the UDS connection fails.

## Scope
- Update core-host to create and listen on a unique UDS socket (e.g., /var/run/tachyon/host-<id>.sock) upon startup.
- Implement the "Shared Discovery Directory" logic (using a shared volume in K8s).
- Refactor the outbound Mesh client to support dual-transport (TCP or UDS) while maintaining the same application-level protocol (H2/H3).

## Success Metrics
- Latency between two local pods is reduced by > 50% compared to standard K8s service networking.
- The system automatically handles pod restarts: if a socket file is stale or missing, the host falls back to the network without dropping the request.
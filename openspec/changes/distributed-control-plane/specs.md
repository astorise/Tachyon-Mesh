# Specifications: Control Plane Separation

## 1. The Data Plane (Host Telemetry & Routing)
The `core-host` exposes a simple capability `wasi:tachyon/telemetry#get-load` returning `{ cpu_pressure: u8, ram_pressure: u8, active_tasks: u32 }`.
The host also exposes `wasi:tachyon/routing#update-target(route: string, new_destination: string)`.
The host does NOT run any background monitoring threads itself. It only updates its atomic counters during FaaS instantiation/destruction.

## 2. The Gossip & Steering FaaS (`system-faas-gossip`)
Runs as a background Reactor.
- **Monitoring:** Wakes up every 500ms (or 100ms if pressure is rising). Calls `get-load()`.
- **Gossiping:** Sends UDP or HTTP/3 datagrams to peer nodes to exchange pressure scores.
- **Steering (P2C):** If local CPU > 80%, it selects two random peers. It picks the one with the lowest pressure. It then calls `update-target("my-heavy-faas", "http://10.0.0.5:8080/my-heavy-faas")`. 
- **Hysteresis:** Routes are only restored to local execution when local pressure drops below 70%, preventing route-flapping (ping-pong effect).

## 3. The Tiered Buffer FaaS (`system-faas-buffer`)
If `system-faas-gossip` detects that ALL peers are > 90% saturated, it updates the route target to point to `http://mesh/system-buffer`.
- **Ingress:** This FaaS receives the HTTP request. It saves the payload in its own allocated RAM volume. If RAM is full, it uses `wasi:filesystem` to write the payload to a local `/var/lib/tachyon/spillover` directory. Returns a 202 Accepted.
- **Egress (Replay):** Triggered by a System Cron (Change 034), it periodically checks cluster pressure. When the cluster is healthy, it reads the stored payloads and fires them back into the Mesh.
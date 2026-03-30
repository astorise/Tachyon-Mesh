# Proposal: Change 015 - System FaaS Telemetry

## Context
`core-host` already emits asynchronous request telemetry, but that data is only printed as JSON lines by the background worker. There is no sealed, privileged execution path for a WebAssembly guest to read host telemetry state and expose it as a service-facing metrics endpoint. The current integrity manifest also cannot distinguish a normal business route from a privileged system route.

## Objective
Introduce a minimal "system FaaS" foundation for telemetry. The host will seal per-route roles inside `integrity.lock`, expose a privileged telemetry snapshot import to a dedicated component world, and execute a system-only Prometheus guest on a sealed route. Under high business load, the host will shed system route execution before it steals capacity from normal guest traffic.

## Scope
- Extend the sealed integrity manifest so each route carries a `role` (`user` or `system`).
- Add a privileged `tachyon:telemetry/reader` WIT interface exposed only through a dedicated `system-faas-guest` world.
- Implement a `system-faas-prom` guest that formats host telemetry snapshots as Prometheus text.
- Track active request pressure in `core-host` and reject system guest execution when the host is above a fixed load threshold.
- Cover the security and QoS behavior with automated tests, and update CI/Docker to build the new guest artifact.

## Success Metrics
- A system guest route sealed with role `system` can instantiate and return telemetry metrics through the privileged import.
- The same guest fails to instantiate when the route is sealed as role `user`.
- Under simulated high load, `core-host` rejects the system telemetry route while continuing to prioritize normal sealed guest routes.

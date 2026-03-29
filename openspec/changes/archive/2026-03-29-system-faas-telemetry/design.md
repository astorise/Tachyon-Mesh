# Design: System Telemetry FaaS Foundations

## Summary
This change adds a privileged telemetry path without disturbing the existing `faas-guest` contract used by regular business functions. The integrity manifest becomes the source of truth for route roles, `core-host` gains a telemetry snapshot view and a system-route QoS gate, and a new `system-faas-guest` world allows a dedicated Prometheus component to read host metrics.

## Sealed Route Metadata
- Replace the plain string route list with structured route entries containing `path` and `role`.
- Keep route normalization and duplicate detection in both `tachyon-cli` and `core-host`.
- Treat `user` as the default execution role for existing routes and reserve `system` for privileged telemetry guests.

## Telemetry Snapshot Runtime
- Extend the telemetry runtime to maintain aggregate counters alongside the existing per-request JSON emission.
- Track `active_requests` on the HTTP path so the host can expose a gauge to system guests and decide when to shed privileged traffic.
- Count dropped telemetry events when the channel is saturated, so the system Prometheus guest can report pressure instead of hiding it.

## Privileged Component World
- Keep `faas-guest` unchanged for normal component guests.
- Add a separate `system-faas-guest` world that imports `tachyon:telemetry/reader` and reuses the existing `handler` export.
- Instantiate component guests through the normal or privileged linker based on the sealed route role.

## QoS
- Before executing a `system` route, compare the active request count with a fixed threshold.
- If the threshold is exceeded, reject the system route with `503 Service Unavailable` and skip guest execution.
- Continue to record telemetry for both admitted and shed requests so operators can see the pressure pattern.

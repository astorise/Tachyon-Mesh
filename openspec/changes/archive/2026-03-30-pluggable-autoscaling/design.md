# Design: Pluggable Legacy Autoscaling

## Summary
This change extends the existing System FaaS model so privileged components can read
per-route queue pressure and, when needed, run autonomous background logic on a
fixed tick. The host keeps the current HTTP execution path for normal user and
system routes, while adding a separate background component world for autoscalers
that must preserve guest state across ticks.

## WIT Contracts
- Extend `system-faas-guest` so privileged HTTP handlers can read both telemetry and
  route queue pressure.
- Add a dedicated `background-system-faas` world that imports queue pressure plus a
  minimal outbound HTTP capability and exports `on-tick`.
- Keep `faas-guest` unchanged for normal business functions.

## Host Runtime
- Replace the plain route semaphore map with a route execution control object that
  stores the semaphore plus an atomic pending-waiter count.
- Expose that waiter count to privileged guests as the pending queue size for a
  sealed route.
- Start background workers only for sealed `system` routes whose component can be
  instantiated with the `background-system-faas` world, so deployments without a
  scaler guest pay no recurring autoscaling overhead.
- Keep a dedicated component instance alive inside each background worker so guest
  memory can retain cooldown state across ticks.

## Autoscaling Guests
- `system-faas-keda` is a privileged HTTP component that renders Prometheus metrics
  for the pending queue depth of the legacy route.
- `system-faas-k8s-scaler` is a privileged background component that checks the same
  queue depth every five seconds and issues a mock Kubernetes `PATCH` request once
  the threshold is exceeded, with a six-tick cooldown to approximate thirty
  seconds.

## Testing Strategy
- Add host tests that verify the scaling metrics route returns Prometheus text for a
  sealed legacy route.
- Add host tests that verify the background scaler sends a mock outbound `PATCH`
  request only after the threshold is crossed and respects the cooldown across
  repeated ticks against the same live component instance.
- Update CI and Docker packaging to build and ship the two new System FaaS
  artifacts.

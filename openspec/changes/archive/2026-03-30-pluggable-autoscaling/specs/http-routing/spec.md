## ADDED Requirements

### Requirement: Host exposes route queue depth to privileged autoscaling guests
The `core-host` runtime SHALL track queued waiters for each sealed route
concurrency limiter and expose the current pending queue size through
`tachyon:mesh/scaling-metrics`.

#### Scenario: Waiting requests increase the reported queue depth
- **WHEN** requests are waiting for capacity on `/api/guest-call-legacy`
- **THEN** `core-host` reports the current waiter count as that route's pending
  queue size
- **AND** a privileged System FaaS guest can read that value without inspecting the
  semaphore directly

### Requirement: Host can drive background system autoscaling guests
The `core-host` runtime SHALL start a five-second background tick loop only for
sealed `system` components that implement the `background-system-faas` world, and
it SHALL preserve the component instance across ticks so guest cooldown state stays
in memory.

#### Scenario: No autoscaling guest configured
- **WHEN** the sealed configuration contains no `system` route backed by a
  `background-system-faas` component
- **THEN** `core-host` does not start any autoscaling tick worker

#### Scenario: Background autoscaler patches a mock Kubernetes deployment
- **WHEN** the pending queue size for `/api/guest-call-legacy` rises above the
  autoscaler threshold
- **AND** the sealed configuration includes the system route `/system/k8s-scaler`
- **THEN** `core-host` invokes the guest `on-tick` export every five seconds
- **AND** the guest issues a mock outbound HTTP `PATCH` request for the legacy
  deployment
- **AND** subsequent ticks within the cooldown window do not issue another patch

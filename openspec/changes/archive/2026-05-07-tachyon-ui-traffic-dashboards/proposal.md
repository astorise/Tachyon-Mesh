# Traffic & Resilience Dashboards Implementation

## Why
Tachyon UI needs dedicated configuration dashboards for traffic routing and L7 resilience controls. These dashboards should use the shared configuration foundation so they remain isolated from IAM, avoid global DOM coupling, and present handled backend errors without breaking the App Shell.

## What Changes
- Add `<tachyon-routing-panel>` for path-to-workload route configuration.
- Add `<tachyon-resilience-panel>` for timeout, retry, and circuit breaker threshold configuration.
- Register both panels in the App Shell component registry.
- Connect both panels to the Tauri `apply_configuration` command.
- Add success pulse feedback when configuration is accepted.

## Impact
Operators can configure routing and resilience from terminal-inspired Web Components mounted through the App Shell router. Routing payloads continue to use the strict `TrafficConfiguration` validator, while resilience settings are validated by the Tauri backend before feedback is shown.

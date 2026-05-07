## Context

The web component shell currently renders after IAM authentication but leaves the operator without an immediate high-level telemetry landing view. Domain dashboards are available through navigation, but the authenticated first screen should summarize mesh health at a glance.

## Goals / Non-Goals

**Goals:**
- Add `<tachyon-overview-panel>` as a `TachyonConfigDashboard`-based dashboard.
- Render key mesh telemetry counters in a responsive card grid.
- Animate counters with GSAP when the panel mounts.
- Automatically show the overview panel after `iam:authenticated`.

**Non-Goals:**
- Implement live telemetry subscriptions.
- Replace the domain-specific configuration panels.
- Add new backend telemetry APIs.

## Decisions

- Use static initial telemetry values for the first panel implementation so it can be mounted without additional backend dependencies.
- Register the overview panel through `ComponentRegistry` and route to it from `TachyonAppShell`.
- Use GSAP counter tweens on text content rather than CSS-only animation so numeric values can later be fed by live telemetry.

## Risks / Trade-offs

- Static values can become stale. Mitigation: keep the panel structure compatible with future API-fed values.
- Automatically routing after login changes the initial shell state. Mitigation: keep sidebar navigation available and mark the overview route active.

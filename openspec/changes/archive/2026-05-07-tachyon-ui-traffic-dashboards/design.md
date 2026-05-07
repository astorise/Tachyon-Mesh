# Design: Traffic & Resilience Dashboards

## Overview
Traffic dashboards are native Web Components built on `TachyonConfigDashboard`. Each dashboard renders inside Shadow DOM, uses the shared constructable stylesheet, submits through Tauri IPC, and reports outcomes through the common `showFeedback` helper.

## Routing Panel
`<tachyon-routing-panel>` presents a compact L7 path-to-workload mapper. It collects an inbound path and target workload, then adapts the form data into the existing strict `TrafficConfiguration` payload accepted by `apply_configuration` for the `config-routing` domain.

## Resilience Panel
`<tachyon-resilience-panel>` presents resilience controls for timeout, retry count, and circuit breaker threshold. It submits a typed resilience payload to `apply_configuration` with the `config-resilience` domain.

## App Shell Integration
Both panels are registered in `ComponentRegistry` and mounted dynamically by `TachyonAppShell`. The registry keeps sidebar labels and route-to-tag mapping outside the shell implementation.

## Feedback Animation
Success feedback uses the shared base class pulse animation on `#feedback-zone`, preserving the Zero-Panic behavior while making accepted configuration changes visible.

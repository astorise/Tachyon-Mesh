## Context

The web component shell now exposes the earlier routing, resilience, AI, hardware, identity, RBAC, workloads, and observability dashboards through `TachyonConfigDashboard` and `ComponentRegistry`. The remaining storage, fleet, and supply-chain configuration domains need matching panels to complete the shell coverage.

## Goals / Non-Goals

**Goals:**
- Add dashboards for storage, fleet node selection, and air-gapped supply-chain policy.
- Reuse the established dark slate/cyan form language and `showFeedback` behavior.
- Route panel submissions through the shared `apply_configuration` command.
- Register all three components in the existing shell navigation.

**Non-Goals:**
- Implement full remote persistence or config fetching.
- Replace the asset registry upload workflow.
- Change the underlying WIT contracts.

## Decisions

- Implement each panel as a custom element extending `TachyonConfigDashboard`.
- Keep the forms intentionally small and aligned with the provided change inputs.
- Add lightweight Tauri validation for `storage`, `fleet`, and `supply_chain` domain payloads so panels return deterministic feedback.

## Risks / Trade-offs

- The UI-level payloads are simplified compared with complete WIT records. Mitigation: validate the key operator inputs and keep domain names stable for future backend wiring.
- Supply-chain signature validation cannot prove trust locally. Mitigation: enforce a digest-like `sha256:` shape before accepting air-gapped policy updates.

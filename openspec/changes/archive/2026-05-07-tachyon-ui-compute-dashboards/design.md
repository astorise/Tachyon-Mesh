## Context

The Tachyon web component shell already supports domain dashboards through `TachyonConfigDashboard`, `ComponentRegistry`, and dynamic route mounting in `TachyonAppShell`. Workload orchestration and observability WIT domains exist but do not yet have dedicated shell panels.

## Goals / Non-Goals

**Goals:**
- Add a Workloads & Secrets dashboard for runtime engine selection and secret reference binding.
- Add an Observability dashboard for OTLP endpoint and log level configuration.
- Register both dashboards in the existing web component shell navigation.
- Route submissions through the shared `apply_configuration` Tauri command.

**Non-Goals:**
- Implement a full workload deployment wizard.
- Persist local drafts or add remote config fetching.
- Change the workload or observability WIT contracts.

## Decisions

- Build both panels as custom elements extending `TachyonConfigDashboard` to reuse styles, animation, and feedback behavior.
- Use `resilientInvoke("apply_configuration", ...)` to keep panel behavior aligned with existing domain dashboards.
- Add lightweight local validation in the Tauri command so the panels return useful feedback even before a remote config API is wired through.

## Risks / Trade-offs

- The panels currently submit compact UI payloads rather than full WIT records. Mitigation: the backend maps and validates the UI-level fields without changing WIT contracts.
- OTLP endpoint validation is intentionally conservative. Mitigation: accept empty endpoints as disabled telemetry and require HTTP(S) when provided.

## Context

The web component shell already routes to domain dashboards through `ComponentRegistry` and renders panels that extend `TachyonConfigDashboard`. Security and identity configuration domains are present in WIT but not exposed in the shell navigation.

## Goals / Non-Goals

**Goals:**
- Add an Identity & Quotas dashboard for JWT issuer and distributed CRDT quota configuration.
- Add an RBAC dashboard for role and policy payload submission.
- Register both dashboards in the web component shell and sidebar routing.

**Non-Goals:**
- Implement a full remote policy editor with persisted drafts.
- Replace the existing authenticated Identity account view.
- Change the backend WIT contracts.

## Decisions

- Build both dashboards as custom elements extending `TachyonConfigDashboard`.
- Use existing `resilientInvoke("apply_configuration", ...)` plumbing to keep behavior consistent with routing, resilience, AI, and hardware panels.
- Register the panels through `ComponentRegistry` so the shell sidebar remains data-driven.

## Risks / Trade-offs

- The panels depend on backend `apply_configuration` domain support. Mitigation: use explicit domain payloads and surface backend validation errors through `showFeedback`.
- JSON/YAML policy content can be invalid. Mitigation: validate JSON client-side before invoking the backend and report parse failures inline.

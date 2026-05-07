# Design: UI Configuration Foundation

## Overview
The configuration foundation introduces a small Web Component framework for Tachyon UI dashboards. The goal is to keep each configuration domain isolated in Shadow DOM while reusing stylesheet, feedback, and mounting behavior.

## Base Component
`TachyonConfigDashboard` extends `HTMLElement` and owns an open Shadow DOM. Subclasses render content through `renderTemplate(html)` so dashboard markup stays scoped to the component and does not depend on global document selectors.

The base class also provides:
- `applyStyles()` to attach the shared constructable stylesheet.
- `showFeedback(type, message)` to render success and error states inside `#feedback-zone`.
- `animateEntrance()` for GSAP-based panel entrance animation.

## Shared Stylesheet
`src/styles/shared-sheets.ts` exports a shared `CSSStyleSheet` created from the existing Tachyon UI Tailwind CSS import. Components reuse the same sheet instance through `shadowRoot.adoptedStyleSheets` to avoid duplicated style text per dashboard instance.

## Component Registry
`src/registry/ComponentRegistry.ts` maps App Shell route slugs to dashboard custom element tags. `TachyonAppShell` resolves the selected route through the registry and mounts the registered custom element in `#router-view`.

Unknown route slugs render a handled feedback state instead of throwing or leaving the shell blank.

## Error Handling
All dashboard command results use `showFeedback`. A handled Rust `Result::Err` or Tauri rejection is displayed inline in the component, preserving the Zero-Panic behavior and keeping the App Shell mounted.

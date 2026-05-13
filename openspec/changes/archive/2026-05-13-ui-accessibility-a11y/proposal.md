# Proposal: UI Accessibility (a11y) Overhaul

## Context
The recent P1 usability audit flagged a severe lack of accessibility features in Tachyon-UI. With only 52 ARIA attributes across the entire application, no semantic HTML structuring, missing focus indicators, and form labels reduced to simple placeholders, the application is currently unusable for users relying on keyboard navigation or screen readers.

## Problem
1. **Keyboard Navigation:** Users cannot visually track where they are tabbing because native outlines have been suppressed without replacement focus rings.
2. **Screen Readers:** Dynamic metrics (telemetry) update silently. Modals do not trap focus or announce themselves as dialogs. Input fields in the IAM suite lack explicit `<label>` associations.
3. **Div Soup:** The core layout relies heavily on generic `<div>` tags instead of semantic landmarks like `<nav>`, `<main>`, and `<section>`, making the page structure opaque to assistive technologies.

## Proposed Solution
1. **Semantic Landmarks:** Refactor `TachyonAppShell.ts` to use proper HTML5 semantic tags.
2. **Global Focus Rings:** Implement standard Tailwind v4 focus utilities across all interactive elements (`button`, `input`, `a`).
3. **ARIA Integration:**
   - Add `aria-live="polite"` to telemetry wrappers (e.g., `TachyonObservabilityPanel`, `NetworkStatus`).
   - Add `role="dialog"`, `aria-modal="true"`, and keyboard traps to all overlays.
4. **Explicit Labeling:** Refactor the auth and MFA forms (`TachyonIAM.ts`, `TachyonMfaPrompt.ts`) to pair every `<input>` with a matching `<label for="...">`.

## Impact
- **Compliance & Usability:** Brings the UI closer to WCAG 2.1 AA standards, ensuring a professional, inclusive experience for all developers and operators.
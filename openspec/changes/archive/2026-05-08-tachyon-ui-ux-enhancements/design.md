## Context

The Tachyon UI shell is implemented as native Web Components with Shadow DOM boundaries. Localization and guided onboarding need to work without adding framework-level dependencies.

## Decisions

- Add a small dictionary-based `i18n.ts` utility with `en` and `fr` dictionaries, localStorage persistence, and a global `i18n:language-changed` event.
- Keep translations as dot-notated keys so components can adopt them incrementally.
- Mount `<tachyon-guided-tour>` inside `<tachyon-app-shell>` so the tour can query and highlight shell elements within the same ShadowRoot.
- Use GSAP for highlight and dialog transitions, matching the rest of the UI animation stack.
- Store tour completion in `localStorage` under `tachyon_tour_completed`, with a header Help/Tour button to relaunch it.

## Trade-offs

- The tour focuses on shell-level controls and dashboard highlights. It does not attempt to walk through every domain panel yet.
- Runtime route labels are translated in the shell rather than changing the component registry contract.

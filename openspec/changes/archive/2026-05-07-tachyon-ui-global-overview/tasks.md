# Tasks: Global Telemetry Overview

- [x] Create `src/components/domains/TachyonOverviewPanel.ts`.
- [x] Implement the HTML grid and GSAP counter animations.
- [x] Register `<tachyon-overview-panel>` in `ComponentRegistry.ts`.
- [x] **Crucial Wire-up**: Update `TachyonAppShell.ts` so that `<tachyon-overview-panel>` is automatically injected into `#router-view` immediately after a successful login (the `iam:authenticated` event).

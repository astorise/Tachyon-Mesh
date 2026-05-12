# Implementation Tasks

- [x] **Task 1: Tauri Configuration**
  - Locate `tachyon-ui/tauri.conf.json`.
  - Replace `"csp": null` with the strict policy defined in the specification.

- [x] **Task 2: Refactor TachyonIAM.ts**
  - Audit `tachyon-ui/src/components/iam/TachyonIAM.ts`.
  - Replace all occurrences of `.innerHTML` that include dynamic variables with `textContent` or standard DOM node creation.

- [x] **Task 3: Global DOM Audit**
  - Run a codebase search for `.innerHTML` in `tachyon-ui/src/components/**`.
  - Refactor all remaining instances, particularly in `TachyonAppShell.ts` and routing components.

- [x] **Task 4: Quality Assurance**
  - Recompile the UI (`cargo tauri build` / `npm run dev`).
  - Verify that the login, MFA sealing, and cluster overview workflows function correctly without CSP violations in the browser console.
  - Verify that GSAP animations and Tailwind styling remain unaffected.
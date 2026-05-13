# Implementation Tasks

- [x] **Task 1: IAM Refactoring**
  - Create `TachyonAuthStepCredentials.ts`.
  - Move the PAT and Stronghold password input HTML/logic from `TachyonIAM.ts` to this new file.
  - Refactor `TachyonIAM.ts` to listen to custom events from the credentials component and trigger the actual authentication flow.

- [x] **Task 2: AppShell Navigation Extraction**
  - Create `TachyonAppShellNav.ts`.
  - Migrate the sidebar HTML, routing click listeners, and active class toggling from `TachyonAppShell.ts`.

- [x] **Task 3: AppShell Modal Extraction**
  - Create `TachyonAppShellModalRoot.ts`.
  - Migrate modal state arrays and render logic out of `TachyonAppShell.ts`.
  
- [x] **Task 4: Integration & QA**
  - Update imports in `main.ts` to ensure the new custom elements are registered.
  - Verify that the login flow (Credentials -> MFA) works smoothly.
  - Verify that clicking a link in the separated Navigation component successfully switches the main content panel.

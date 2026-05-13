# Proposal: UI Component Decomposition (AppShell & IAM)

## Context
The recent P1 usability and architecture audit identified `TachyonAppShell.ts` (1000+ LOC) and `TachyonIAM.ts` (600+ LOC) as architectural bottlenecks. These monolithic Vanilla Web Components currently handle too many responsibilities, blending UI rendering, modal orchestration, state management, and complex authentication flows.

## Problem
1. **Testing:** It is virtually impossible to unit test the credentials form without also spinning up the MFA flow and the surrounding IAM shell.
2. **Maintenance:** Any minor change to the sidebar navigation risks breaking the modal rendering logic, as both reside within `TachyonAppShell.ts`.
3. **Reusability:** UI elements like the navigation sidebar and the modal root are locked inside the shell and cannot be easily reused or mocked.

## Proposed Solution
Following the audit's recommendation, we will decompose these monoliths into highly cohesive, single-responsibility sub-components:
1. **IAM Decomposition:** Split `TachyonIAM.ts` into a controller component that orchestrates two new dumb components: `<auth-step-credentials>` and `<auth-step-mfa>` (refining the existing `TachyonMfaPrompt`).
2. **Shell Decomposition:** Split `TachyonAppShell.ts` into `<app-shell-nav>` (handling the sidebar and routing logic) and `<app-shell-modal-root>` (handling the overlay and dialog stacking).

## Impact
- **Testability:** Smaller components mean we can finally introduce isolated DOM tests for the login flow.
- **Maintainability:** Reduced LOC per file and strict separation of concerns.
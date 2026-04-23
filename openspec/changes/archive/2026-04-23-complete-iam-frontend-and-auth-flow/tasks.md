# Tasks: complete-iam-frontend-and-auth-flow

- [x] Rewrite the malformed change artifacts into the current OpenSpec layout with valid proposal sections and delta specs under `specs/`.
- [x] Replace the legacy `#connection-overlay` in `tachyon-ui/index.html` with a two-step `#auth-overlay` and upgrade `#view-identity` into a routed IAM dashboard.
- [x] Extend `tachyon-ui/src/main.ts` to drive the AuthN login-to-MFA state machine, render IAM data, and bind Identity security actions.
- [x] Add the minimal Tauri/client command surface required by the new frontend flow without introducing unsupported remote IAM CRUD endpoints.
- [x] Validate with OpenSpec, frontend build, workspace build/tests, CI-equivalent checks, then commit, push, deploy, and archive the change.

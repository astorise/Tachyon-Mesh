# Title: Comprehensive IAM Frontend, Auth Flow & Step-up Authentication

## Problem Statement
The current IAM UI suffers from severe onboarding friction and security gaps, as highlighted in the latest audit:
1. **Missing Token Generation:** There is no Admin UI to generate invite tokens (`POST /admin/enrollment/start`); it relies on out-of-band CLI commands.
2. **CA Persistence:** Custom CA certificates must be re-selected manually on every login, lacking persistent management.
3. **Insecure Storage:** `localStorage` is used for credentials.
4. **Unprotected Writes:** While read-access should be seamless via a stored PAT, sensitive operations (like "Seal & Apply") lack a Step-up Authentication (Sudo mode) barrier.
5. **Incomplete Guided Tour:** The onboarding tour misses the critical path entirely.

## Objective
Implement a unified, highly secure, and frictionless IAM flow:
1. Migrate credential and CA persistence to `tauri-plugin-stronghold`.
2. Implement Step-up Authentication (Sudo mode) with a 20-minute grace period for write operations.
3. Build the Admin "Generate Invite Token" panel in `<tachyon-iam>`.
4. Update the Guided Tour to cover the `login -> first-run MFA -> seal & apply` path.
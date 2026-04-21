# Tasks: Change 069.1 Implementation

**Agent Instruction:** Integrate the recovery code generation and consumption logic into the auth FaaS and update the UI onboarding flow. Maintain 4-space indentation for code.

- [x] Extend `system-faas-auth` with `generate-recovery-codes` and `consume-recovery-code`, including persistent hashed storage for each username.
- [x] Issue an emergency JWT when a valid recovery code is consumed and burn the used code immediately.
- [x] Add protected and public host endpoints so the desktop client can request onboarding recovery codes and later redeem a code for emergency access.
- [x] Add a first-run onboarding modal in `tachyon-ui`, reveal the recovery-code step after the QR placeholder step, and render the generated codes.
- [x] Require downloading the `tachyon-recovery-codes.txt` file before allowing the onboarding modal to complete.

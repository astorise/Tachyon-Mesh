# Proposal: Token-based Signup and Mandatory 2FA Enrollment

## Context
Tachyon Mesh currently supports 2FA (TOTP) during the authentication phase, but it lacks the UI and flow for the initial user enrollment and 2FA setup. As a Zero-Trust platform, we cannot allow open registrations. 

## Proposed Solution
We will implement an "Invite-Only" onboarding flow. An administrator will generate a one-time use Registration Token (with a 24-hour TTL, leveraging our existing CI/CD token infrastructure). The user will use this token to access the signup flow.

The flow will strictly follow these steps:
1. **Token Verification:** User enters the Registration Token. The system validates it and retrieves the pre-assigned role/rights.
2. **Profile Setup:** User enters their First Name, Last Name, Login (Username), and Password.
3. **2FA Enrollment:** The system generates a TOTP secret, displays a QR Code, and prompts the user for the first 6-digit code to finalize the account creation.

## Objectives
- Eliminate open registration endpoints.
- Ensure no user can exist in the system without 2FA fully configured.
- Absorb the AI Debt on the `tachyon-ui` side regarding the authentication loop.
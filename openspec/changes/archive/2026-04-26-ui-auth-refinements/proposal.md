# Proposal: UI Auth Refinements (Signin & Signup UX)

## Context
Following the implementation of the Token-based Signup and 2FA enrollment, a review of the `tachyon-ui` authentication flows revealed minor UX and logical inconsistencies that need to be addressed before the final release.

## Proposed Solution
1. **Login View Simplification:** The `LoginView` currently prompts for a Token. Since tokens are strictly one-time-use for the `Signup` phase (Invite Token), this field is confusing during regular Signin and must be removed.
2. **Signup Username Standardization:** To maintain a clean directory of identities, the `username` (login) field should no longer be a free-form input. It will be automatically computed and formatted as `firstname.lastname` (strictly lowercase, stripping special characters/spaces).
3. **Password Safety:** Users currently type their password blindly during Step 2 of the Signup wizard. We will add a "Confirm Password" field to prevent typos, along with a "Toggle Visibility" (eye icon) feature for all password inputs across the application.

## Objectives
- Reduce friction during standard logins.
- Enforce strict naming conventions for user identities.
- Prevent user lockouts due to password typos during onboarding.
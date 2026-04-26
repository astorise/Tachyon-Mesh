# Design: Auth UX Updates

## 1. Login View (`tachyon-ui/src/views/LoginView.tsx`)
- Remove the `token` input field and its associated state.
- The view should strictly contain: `Username`, `Password`, and the "Sign In" action, alongside the secondary "Register with Invite Token" navigation button.
- Implement a state toggle `showPassword` (boolean) to switch the password input's `type` attribute between `"password"` and `"text"`. Include an eye icon inside the input field.

## 2. Signup Profile Entry (`tachyon-ui/src/views/ProfileEntry.tsx`)
- **Username Computation:** - Watch the `firstName` and `lastName` state.
  - Implement a helper function `formatUsername(first, last)` that converts to lowercase, replaces spaces with dots, and removes diacritics/accents.
  - The `Username` field should be `readOnly` or `disabled`, acting as a visual confirmation of the computed identity.
- **Password Confirmation:**
  - Add a new state/field: `confirmPassword`.
  - The "Next" button must remain disabled until `password === confirmPassword` and both meet the minimum security strength.
  - Apply the same "Toggle Visibility" (eye icon) logic to both password fields.

## 3. State Management (`tachyon-ui/src/stores/signupStore.ts`)
- Add `confirmPassword` to the temporary state to handle validation, though it does NOT need to be sent to the backend during `stage-user`.
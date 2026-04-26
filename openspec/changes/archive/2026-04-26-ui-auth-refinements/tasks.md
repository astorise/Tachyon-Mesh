# Implementation Tasks

## Phase 1: Login View Cleanup
- [x] Open `LoginView.tsx` and remove the Token input field.
- [x] Add an interactive "eye" icon (using an SVG or a library like `lucide-react`) to the Password field to toggle input type.

## Phase 2: Profile Entry Logic (Signup)
- [x] Open `ProfileEntry.tsx`.
- [x] Add the auto-formatting logic for the Username field: `(firstName + "." + lastName).toLowerCase()`. Disable manual edits on this field.
- [x] Add the `Confirm Password` input field.
- [x] Add the password visibility toggle to both password fields.

## Phase 3: Validation Updates
- [x] Update the `SignupStore` or the local form validation logic to ensure `password === confirmPassword`.
- [x] Show a clear inline error message (e.g., text in red) if the passwords do not match when the user attempts to proceed to the TOTP setup.

## Phase 4: Polish
- [x] Ensure all input fields maintain consistent Tailwind CSS styling, especially when appending the absolute-positioned "eye" icon.
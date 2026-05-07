# Tasks: IAM Migration (Phase 1)

## Setup & Foundations
- [x] Create `src/components/iam/TachyonIAM.ts` and define the class extending `HTMLElement`.
- [x] Implement Tailwind CSS import and configure Constructable Stylesheets.
- [x] Register the component using `customElements.define('tachyon-iam', TachyonIAM)`.

## Logic Migration
- [x] Move the existing Login/Signup HTML structure from `index.html` to the component's template.
- [x] Migrate GSAP animations, ensuring the use of `this.shadowRoot.querySelector(...)` instead of global `document.querySelector`.
- [x] Encapsulate Tauri IPC calls (`invoke`) within the component's form validation logic.
- [x] Dispatch the `iam:authenticated` event upon success.

## Integration
- [x] Clean up `index.html` by replacing static divs with `<tachyon-iam id="auth-layer"></tachyon-iam>`.
- [x] Update `main.ts` to listen for `document.addEventListener('iam:authenticated', ...)` and log the payload.

## QA Validation
- [x] Verify that the login button correctly triggers the Rust backend.
- [x] Verify *Zero-Panic* error handling: wrong password gracefully displays an error within the component.
- [x] Ensure global CSS classes do not leak into or break the component's styling.

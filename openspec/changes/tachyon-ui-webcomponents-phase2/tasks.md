# Tasks: App Shell Migration (Phase 2)

## Component Creation
- [x] Create `src/components/layout/TachyonAppShell.ts` and set up the boilerplate.
- [x] Implement Tailwind Constructable Stylesheets injection.

## Template Integration
- [x] Define internal HTML structure (Sidebar, Header, Router View).
- [x] Apply "Dark Slate / Cyan" Tailwind classes.

## Logic and Animation
- [x] In `connectedCallback()`, add a document listener for `iam:authenticated`.
- [x] Implement the `startTransition(userData)` method containing the GSAP Timeline.
- [x] Update the Header DOM dynamically with `userData.user`.

## Integration & QA
- [x] Add `<tachyon-app-shell>` to `index.html` right after `<tachyon-iam>`.
- [x] Verify that on page load, only IAM is visible.
- [x] Verify that successful login triggers the GSAP transition seamlessly.

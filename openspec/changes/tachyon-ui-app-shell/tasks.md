# Execution Tasks for Codex

- [ ] **Task 1 (HTML Skeleton)**: Overwrite `tachyon-ui/index.html` using the HTML structure defined in `design.md`, applying the exact Tailwind slate/cyan color classes provided.
- [ ] **Task 2 (Router Logic)**: Create `tachyon-ui/src/router.ts`. Implement the Vanilla JS Router class relying solely on `hashchange` events. Ensure it emits a `route-change` CustomEvent.
- [ ] **Task 3 (GSAP Integration)**: Create `tachyon-ui/src/animations.ts`. Implement the GSAP logic: a staggered entrance for the sidebar navigation links on boot, and a cross-fade/slide-up transition for the main content area when the `route-change` event fires.
- [ ] **Task 4 (Main Entry)**: Update `tachyon-ui/src/main.ts` to instantiate the `Router` and trigger the initial `handleRoute()` call on startup.
- [ ] **Task 5 (OpenSpec)**: Create the `specs/tauri-configurator/spec.md` delta to formalize the Zero-Overhead frontend architecture.
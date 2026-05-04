# Execution Tasks for Codex

- [x] **Task 1 (View Module)**: Create `tachyon-ui/src/views/routing.ts` and paste the HTML template function `renderRoutingView` defined in `design.md`.
- [x] **Task 2 (Controller Module)**: Create `tachyon-ui/src/controllers/routingController.ts` and implement the `RoutingController` class with the event listeners.
- [x] **Task 3 (Router Integration)**: Update `tachyon-ui/src/router.ts`. Import `renderRoutingView` and `RoutingController`. Update the `/routing` route to use `renderRoutingView()`. In the `handleRoute` function, after dispatching the `route-change` event (or immediately after DOM injection), call `RoutingController.init()` if the active route is `/routing`.
- [x] **Task 4 (OpenSpec)**: Create the `specs/tauri-configurator/spec.md` delta to formalize that configuration views must generate payloads strictly matching the GitOps schemas.

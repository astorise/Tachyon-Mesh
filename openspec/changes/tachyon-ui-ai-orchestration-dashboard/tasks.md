# Execution Tasks for Codex

- [ ] **Task 1 (View Module)**: Create `tachyon-ui/src/views/aiOrchestration.ts` and paste the HTML template function `renderAiOrchestrationView` defined in `design.md`.
- [ ] **Task 2 (Controller Module)**: Create `tachyon-ui/src/controllers/aiController.ts` and implement the `AiOrchestrationController` class to handle animations and data binding.
- [ ] **Task 3 (Router Integration)**: Update `tachyon-ui/src/router.ts`. Add a new sidebar link in the HTML skeleton (`<a href="#/ai-models"...>AI Models</a>`). Map the route `'/ai-models'` to `renderAiOrchestrationView()`. In the router logic, call `AiOrchestrationController.init()` after the view is injected.
- [ ] **Task 4 (OpenSpec)**: Create the `specs/tauri-configurator/spec.md` delta to document the UI's role in configuring multi-gpu and LoRA multiplexing dynamically.
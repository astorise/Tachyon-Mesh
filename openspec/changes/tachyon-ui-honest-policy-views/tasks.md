## 1. Shared Policy Form badge component

- [x] 1.1 Create `tachyon-ui/src/components/base/TachyonPolicyFormBadge.ts` as a Shadow DOM custom element using the existing `tachyonSharedStylesheet`
- [x] 1.2 Render a slate chip with the label sourced from i18n key `policy-form-badge.label`
- [x] 1.3 Wire a native `title` attribute (or accessible tooltip) populated from `policy-form-badge.tooltip`
- [x] 1.4 Register the element via `customElements.define("tachyon-policy-form-badge", ...)` and subscribe to `i18n:language-changed` for re-render
- [x] 1.5 Add i18n entries for `policy-form-badge.label` ("Policy form") and `policy-form-badge.tooltip` ("Writes configuration — does not display the cluster's current state") in every locale that currently exists in `tachyon-ui/src/utils/i18n.ts`

## 2. Embed the badge in policy panels

- [x] 2.1 Insert `<tachyon-policy-form-badge>` in the header of `TachyonResiliencePanel.ts`
- [x] 2.2 Insert `<tachyon-policy-form-badge>` in the header of `TachyonIdentityPanel.ts`
- [x] 2.3 Insert `<tachyon-policy-form-badge>` in the header of `TachyonRbacPanel.ts`
- [x] 2.4 Insert `<tachyon-policy-form-badge>` in the header of `TachyonSupplyChainPanel.ts`
- [x] 2.5 Insert `<tachyon-policy-form-badge>` in the header of `TachyonFleetPanel.ts`
- [x] 2.6 Import the badge module from each panel (so the customElement is registered before mount)
- [x] 2.7 Add a unit test per panel asserting the badge is present in the rendered shadow tree
- [x] 2.8 Add a negative unit test on `TachyonOverviewPanel`, `TachyonNodesPanel`, `TachyonSystemsPanel`, `TachyonTopologyPanel`, `TachyonUsersPanel`, `TachyonWorkloadsPanel`, `TachyonObservabilityPanel`, `TachyonStoragePanel`, `TachyonAIPanel` asserting the badge is NOT present

## 3. Topology View / Edit mode

- [x] 3.1 Add a `mode: "view" | "edit"` field to `TachyonTopologyPanel`, defaulting to `"view"`
- [x] 3.2 On `connectedCallback`, hydrate the field from `sessionStorage.getItem("tachyon-ui:topology-mode")` (fallback `"view"` if absent or invalid)
- [x] 3.3 Render a header toggle button group `[View | Edit]` and wire click handlers that update the field, persist to `sessionStorage`, and trigger `this.render()` + `this.bindEvents()` + `this.pushGraphToCanvas()`
- [x] 3.4 Conditionally render `#add-node-form` and `#btn-apply-topology` only when `mode === "edit"`
- [x] 3.5 In `TachyonTopologyCanvas`, accept an `editable: boolean` property; when `false`, suppress `pointerdown` handlers on `[data-node-id]` and reject `topology:wasm-dropped` drop events
- [x] 3.6 In `TachyonNodeEditor`, accept an `editable: boolean` property; when `false`, render a read-only summary instead of the form (label + key/value table, no Save/Delete buttons)
- [x] 3.7 In `TachyonTopologyPanel`, pass the current mode through to canvas and editor via attributes/properties
- [x] 3.8 Append the localised mode suffix (`topology.mode.view` / `topology.mode.edit`) to the existing live/offline banner text
- [x] 3.9 Add i18n entries for `topology.toggle.view`, `topology.toggle.edit`, `topology.mode.view`, `topology.mode.edit` in every locale
- [x] 3.10 Update existing unit tests for the topology panel to default-assert View-mode rendering; add new tests for the Edit-mode rendering and the session persistence

## 4. Playwright coverage for previous-change gaps

- [x] 4.1 Create `tachyon-ui/e2e/topology-empty-state.spec.ts` that fakes `get_topology_graph` to return an empty graph and asserts the empty-state card is rendered with its CTA
- [x] 4.2 In the same spec, assert that zero `[data-node-id]` elements exist in the canvas
- [x] 4.3 Create `tachyon-ui/e2e/topology-demo-flag.spec.ts` that loads the app with `?demo=1`, fakes an empty `get_topology_graph`, and asserts the "Demo data" banner plus at least one sample node
- [x] 4.4 Wire both specs into the existing `tachyon-ui/playwright.config.ts` so they run in the default project

## 5. Verification and documentation

- [x] 5.1 Run `npm run test` (Vitest) in `tachyon-ui/` and confirm green
- [ ] 5.2 Run `npm run e2e` (Playwright) in `tachyon-ui/` and confirm green
- [ ] 5.3 Manually verify the badge appears with correct copy on each of the five policy panels
- [ ] 5.4 Manually verify Topology defaults to View, toggle persists across reload, banner shows the mode suffix
- [x] 5.5 Update `CHANGELOG.md` with a one-line summary under the unreleased section
- [x] 5.6 Verify `openspec status --change "tachyon-ui-honest-policy-views"` shows all artifacts done and `isComplete = true`

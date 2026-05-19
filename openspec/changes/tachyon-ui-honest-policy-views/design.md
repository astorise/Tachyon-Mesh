# Design: Honest Policy Views and Topology Read-Only Mode

## Context

After `mesh-node-registry-and-systems-catalog`, Tachyon-UI exposes two new state views (Nodes, Systems) and a corrected Overview / Topology empty state. Two issues from the original audit remain visible in the running app:

- The six panels with the `*PolicyDashboard` shape (`TachyonResiliencePanel`, `TachyonIdentityPanel`, `TachyonRbacPanel`, `TachyonSupplyChainPanel`, `TachyonFleetPanel`) render only `<form>` controls and a "Apply" button, with no current-state section. An operator opening "RBAC" expects to see the current rules; they see an empty JSON template instead. There is no visual cue that the screen is write-only.
- `TachyonTopologyPanel` mixes a read of `get_topology_graph` with an editable canvas. Drag, drop, add-node and "Apply Topology" mutate state that the operator may have thought was just an observation. The `?demo=1` flag and the empty-state card now make the *empty* case honest, but the populated case still conflates view and edit.

Both issues are UI-only. No backend command is missing for the badge work; the topology mode toggle is also a local-only change. Backend extensions to actually surface current policies (RBAC rules, supply chain signers, resilience budgets) belong to dedicated domain changes — not this one.

## Goals / Non-Goals

**Goals:**
- Make it immediately obvious that the six policy panels write configuration and do not show current state.
- Make the topology canvas honest about whether the operator is looking at the mesh or composing a spec.
- Cover the two Playwright gaps inherited from the previous change (`12.5` topology empty state, `12.6` demo flag).

**Non-Goals:**
- Building backend commands that would let the policy panels actually display current state (per-domain follow-ups).
- Replacing `<tachyon-topology-panel>` with two distinct routes.
- Reworking the AI and Workloads panels which are partially stateful — they get a smaller treatment (no badge today since they already show some state); a future change can revisit.
- Adding config-history / audit views on the policy panels.

## Decisions

### D1. Single shared `<tachyon-policy-form-badge>` component

**Decision.** Implement the badge once as a tiny web component in `tachyon-ui/src/components/base/TachyonPolicyFormBadge.ts`. Each policy panel inserts `<tachyon-policy-form-badge></tachyon-policy-form-badge>` in its header next to the title, and the badge owns its own styling, hover tooltip, and i18n lookup.

**Why.** The badge needs to land in five panels; five copies of the same Tailwind chip is the textbook reason to extract a component. The shared sheet (`shared-sheets`) already exists for that purpose. A component also lets a future change wire a click target (e.g. "Open the related Nodes / Audit view") without re-touching every host panel.

**Alternatives considered.** (a) Inline span with Tailwind classes per panel — rejected (five copies, easy to drift). (b) A shared helper function returning HTML — rejected (loses Shadow DOM isolation and i18n re-render on language change).

### D2. View / Edit mode in `TachyonTopologyPanel`, default to View

**Decision.** Add a header toggle button group `[ View | Edit ]` to `<tachyon-topology-panel>`. Default mode is **View** on mount. In View mode: zoom and pan still work; node-drag, add-node form, node-editor "Save"/"Delete", `topology:wasm-dropped`, and the "Apply Topology" button are all disabled or hidden. Switching to Edit re-enables them. The mode is persisted to `sessionStorage` under `tachyon-ui:topology-mode` so a page refresh keeps the operator's choice; cross-session persistence is deliberately not added.

**Why.** The hazard is the operator believing they observe state and accidentally mutating it. Defaulting to View makes the safe path the default; an explicit click is required to enter the editing surface. `sessionStorage` (not `localStorage`) is chosen so a future operator using the same browser does not inherit the previous one's mode.

**Trade-off.** A loud toggle adds a piece of chrome to every topology page load. We accept that — the existing canvas chrome already has zoom / compact / reset controls, so one more button is in keeping.

### D3. Mode is reflected in the source banner

**Decision.** The existing banner that reads `topology.live-banner` or `topology.offline-banner` is appended with " · View" or " · Edit". The render path stays identical except for one i18n string concatenation.

**Why.** A toggle button can be missed when the operator is focused on the canvas itself. The banner is at the top of the panel header, in the field of view whenever the operator looks at the title — it is the right place to confirm the current mode at a glance.

### D4. No new Tauri commands, no new `tachyon-client` functions

**Decision.** This change touches no Rust code. Every modification lives under `tachyon-ui/src/`.

**Why.** Per the audit, the substantive backend gap was filled by `mesh-node-registry-and-systems-catalog`. The remaining UI honesty work has no backend dependency: it is a labelling and mode-gating problem.

### D5. Playwright covers the two gaps inherited from the previous change

**Decision.** Add two Playwright specs:
- `tachyon-ui/e2e/topology-empty-state.spec.ts` — boots the UI against a fake `get_topology_graph` returning `{nodes:[], edges:[]}`, asserts the empty-state card is rendered with its CTA, and asserts no `[data-node-id]` button is present.
- `tachyon-ui/e2e/topology-demo-flag.spec.ts` — boots the UI with `?demo=1` against the same empty backend, asserts the demo banner and at least one sample node are rendered.

**Why.** These were tasks 12.5 and 12.6 in the previous change's `tasks.md`, left unchecked because they required manual verification. With a Playwright spec they become CI-tracked.

## Risks / Trade-offs

- **Operators may dislike that Topology defaults to View** → Mitigation: the toggle is one click away and the choice persists for the session. We measure by feedback rather than guess.
- **The badge could be misread as a warning** → Mitigation: copy is neutral ("Policy form — writes configuration, does not display current state"). No red colouring; use the existing slate badge style.
- **`sessionStorage` is per-tab** → Acceptable. Operators routinely use one tab.
- **Adding the badge to `TachyonFleetPanel` is awkward now that Nodes lives elsewhere** → That is precisely the point: Fleet *is* a policy form. The badge makes it honest; a future change can rename the route to "Fleet Policy" without touching the badge.

## Open Questions

None. The change is small enough to be self-contained.

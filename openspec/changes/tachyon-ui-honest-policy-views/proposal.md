# Proposal: Honest Policy Views and Topology Read-Only Mode

## Why

The audit that led to `mesh-node-registry-and-systems-catalog` flagged two issues that change deliberately left out of scope:

1. **Six panels are pure policy forms that pretend to be domain views.** `TachyonResiliencePanel`, `TachyonIdentityPanel`, `TachyonRbacPanel`, `TachyonSupplyChainPanel`, `TachyonFleetPanel`, and parts of `TachyonAIPanel` and `TachyonWorkloadsPanel` render headers like "Resilience" / "RBAC" / "Supply Chain" with no current-state information. An operator reasonably expects "RBAC" to show the current rules; instead they see an empty textarea with a default JSON template. The visual treatment hides the fact that these screens are write-only.

2. **`TachyonTopologyPanel` conflates two responsibilities.** It is both a *view of the current mesh topology* (loaded from `get_topology_graph`) and an *editor for a topology spec* (drag, drop, add-node, build-bundle, apply). The same nodes can be read-only data from the backend or in-flight edits, and the UI gives no indication of which. After the previous change removed the `DEFAULT_NODES` fallback, the empty-state path is honest, but the populated path remains ambiguous: edits silently mutate what the operator thought was a live snapshot.

This change makes both problems visible to the operator without yet building the missing backend surfaces (which belong in domain-by-domain follow-ups).

## What Changes

- Add a shared `<tachyon-policy-form-badge>` web component that renders a small "Policy form" chip next to the header of any panel that has no current-state surface. Apply it to the six panels named above.
- Update i18n dictionaries with the badge label and a one-sentence tooltip explaining that the panel writes configuration but does not display the cluster's current state.
- Split `<tachyon-topology-panel>` behaviour into two explicit modes selectable via a header toggle: **View** (read-only, drag still pans/zooms but nodes cannot be moved/added/deleted, no "Apply" button) and **Edit** (current behaviour). Default to View on mount. Persist the mode in `sessionStorage` so a refresh keeps the operator's choice.
- Surface the mode in the existing live/offline banner: `topology.live-banner` and `topology.offline-banner` get a "View" / "Edit" suffix so the mode is visible at a glance.
- Add Playwright specs for the topology empty state (covers ex-task 12.5 from the previous change) and for the `?demo=1` flag (covers ex-task 12.6).

## Capabilities

### New Capabilities
- `tachyon-ui-policy-form-badge`: Shared visual treatment that marks any panel as a write-only policy form, with a tooltip explaining the absence of a current-state view.

### Modified Capabilities
- `topology-canvas-taxonomy`: Add a requirement that the canvas operates in a View / Edit mode toggle, with View as the default and edits gated behind explicit Edit-mode activation. Existing taxonomy, drag, editor, serialize, add/delete requirements are unchanged except that drag/add/delete are now Edit-mode-only.

## Impact

- **Affected files**:
  - `tachyon-ui/src/components/base/TachyonPolicyFormBadge.ts` (new shared web component)
  - `tachyon-ui/src/components/domains/TachyonResiliencePanel.ts`, `TachyonIdentityPanel.ts`, `TachyonRbacPanel.ts`, `TachyonSupplyChainPanel.ts`, `TachyonFleetPanel.ts` (badge insertion)
  - `tachyon-ui/src/components/domains/TachyonTopologyPanel.ts` (mode toggle, edit-gating)
  - `tachyon-ui/src/styles/shared-sheets` and the i18n dictionaries (`tachyon-ui/src/utils/i18n.ts` data)
  - `tachyon-ui/e2e/` (two new Playwright specs)
- **No backend changes.** This change is UI-only.
- **No new Tauri commands.** No new `tachyon-client` functions.
- **Out of scope** (kept for follow-up changes):
  - Adding actual current-state sections to the six policy panels — each requires a new backend command per domain (`list_resilience_policies`, `list_rbac_rules`, etc.). These deserve their own scoped changes.
  - Removing the `<tachyon-topology-panel>` edit mode altogether and moving editing to a dedicated "Topology Editor" route.
  - Surfacing config history / audit on the policy panels.

# Design: ui-xss-mitigation-reapply

## Strategy

Rather than refactor every Shadow DOM `innerHTML` template literal, the migration eliminates **all** `escapeHtml`/`escape`/`escapeAttr` function definitions across the codebase by combining two patterns:

1. **Shared `el()` helper** — `tachyon-ui/src/utils/dom-safe.ts` builds DOM trees via `createElement` + `textContent`. User strings flow through `textContent` and `setAttribute`, never `innerHTML`.

2. **Render-then-populate pattern** — keep static structural HTML (with `t()` translations, which return constants from the trusted i18n dictionary) as `innerHTML` for layout, then populate dynamic user-controlled fields via DOM API after render. For form inputs, the static template leaves `value` blank and a `populateFieldValues()` method sets `(input as HTMLInputElement).value = userData` after render — this uses the input *property*, not the value attribute, so it never reaches HTML parsing.

## Files Refactored

| File | Previous | Now |
|---|---|---|
| `TachyonAppShell.ts` | Unused `escapeHtml()` definition | Removed |
| `TachyonAppShellNav.ts` | `escapeHtml(entry.route)`, `escapeHtml(t(...))` in nav buttons | DOM API via `el()` |
| `TachyonStoragePanel.ts` | `escapeHtml(resource.*)` in table rows | `populateResourceRows()` builds `<tr>` via `el()` |
| `TachyonHardwarePanel.ts` | `this.escape(gpu.model/id)` in VRAM bars | `populateGpuBars()` builds bars via `el()` |
| `TachyonRoutingPanel.ts` | `this.escape(route.name/path)` in snapshot table | `populateSnapshotRows()` |
| `TachyonObservabilityPanel.ts` | `this.escape(metrics.source, line.*, diff.*)` | `populateObservability()` |
| `TachyonWorkloadsPanel.ts` | `this.escape(rollout.*)` in canary rollout list | `populateRollouts()` + `phaseBadgeNode()` |
| `TachyonBundleConflictModal.ts` | `this.escape/escapeAttr(conflict.*)` | `populateConflictRows()` builds conflict `<li>` via `el()` |
| `TachyonUsersPanel.ts` | `this.escape/escapeAttr(user.*, group.*, audit.*)` (20 sites) | `populateUserRows()`, `populateGroupList()`, `populateAuditModal()` |
| `TachyonTopologyPanel.ts` | `this.escape(node.*, edge.id, formValue)` (27 sites across 3 classes) | `renderNode()` returns `HTMLElement`; `populateEdges()` uses `createElementNS`; `populateFieldValues()` sets input `.value` properties |

## Files Verified Clean (Tasks 1 & 2 named files with no escape usage)

- `aiOrchestration.ts`, `routing.ts`, `TachyonRoutingDashboard.ts` — no `innerHTML` or `escape` usage; no changes needed.

## Build Verification

`cd tachyon-ui && npm run build` succeeds with:
- `tsc --noEmit` — zero TypeScript errors
- `vite build` — 74 modules transformed, 363.67 kB output

Zero `escapeHtml`/`escape`/`escapeAttr` function definitions remain in `tachyon-ui/src/`.

# Proposal: Complete i18n Coverage for All Domain Panels

## Why

The `ui-ux-enhancements` change specified that `TachyonAppShell` and
`TachyonOverviewPanel` would consume the i18n utility. Subsequent changes
(`ui-functional-restoration-and-i18n`) extended coverage to the IAM overlay,
MFA prompt, observability, routing, and storage previews. However, eight
domain panels still hardcoded every operator-visible string in English with no
`t()` calls: AI, Hardware, Identity, RBAC, Workloads, Fleet, SupplyChain, and
Resilience.

## What Changes

All eight remaining panels are refactored to:
1. Import `t` from `utils/i18n`.
2. Replace every hardcoded heading, label, option, placeholder, button and
   feedback string with a `t(key)` call.
3. Listen for `i18n:language-changed` and re-render so the language toggle
   takes immediate effect without a page reload.

The `i18n.ts` dictionary is extended with 73 new keys (EN + FR) covering the
full string surface of all eight panels.

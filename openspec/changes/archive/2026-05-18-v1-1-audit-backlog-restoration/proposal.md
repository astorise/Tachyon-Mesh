# Proposal: v1.1.x Audit Backlog Restoration

## Context
During the exhaustive `v1.1.x` audit remediation, several implementation tasks across 8 different proposals were correctly unmarked (`[ ]`) to reflect their true unfinished state. However, because these proposal directories remain inside the `openspec/changes/archive/` folder, the agentic workflow treats them as closed. They will never be picked up for future implementation passes.

## Objective
Restore the incomplete features to the active development pipeline. The goal is strictly administrative file-system movement, ensuring the previously stubbed or un-wired features are re-enqueued as active technical debt to be completed in future `v1.2.0` or `v1.1.x` minor cycles.

## Scope
File system movements from `openspec/changes/archive/` to `openspec/changes/` for the 8 identified unfinished proposals.
# Proposal: Change 051 - Graceful Draining & Zero-Downtime Reload

## Context
When the `integrity.lock` is updated (Change 026), the host currently reloads the configuration. If it destroys active instances of `v1` to deploy `v2`, in-flight HTTP requests are brutally terminated, leading to 502/504 errors. We need a "Draining" phase.

## Objective
1. Implement a dual-state routing during hot-reloads: `Active` (new traffic) and `Draining` (finishing existing traffic).
2. Ensure `v1` instances are only destroyed when their internal request counter reaches zero or a safety timeout is exceeded.

## Success Metrics
- 0% dropped connections during a version upgrade.
- The transition from `v1` to `v2` is invisible to the end-user, even for long-running requests (up to 30s).
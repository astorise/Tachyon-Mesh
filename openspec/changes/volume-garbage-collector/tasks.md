# Tasks: Change 033 Implementation

**Agent Instruction:** Read `proposal.md` and the delta spec under `specs/`. Implement the background garbage collector for ephemeral volumes.

- [ ] Extend the volume schema with optional `ttl_seconds` while preserving backward compatibility.
- [ ] Start a periodic sweeper that deduplicates TTL-managed host paths and performs filesystem scans from blocking workers.
- [ ] Delete stale files and directories based on modified time while handling filesystem races gracefully.
- [ ] Validate automatic cleanup of a short-lived test volume without crashing the host.

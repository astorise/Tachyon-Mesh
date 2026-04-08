# Tasks: Change 033 Implementation

**Agent Instruction:** Read `proposal.md` and the delta spec under `specs/`. Implement the background garbage collector for ephemeral volumes.

- [x] Extend the volume schema with optional `ttl_seconds` while preserving backward compatibility.
- [x] Start a periodic sweeper that deduplicates TTL-managed host paths and performs filesystem scans from blocking workers.
- [x] Delete stale files and directories based on modified time while handling filesystem races gracefully.
- [x] Validate automatic cleanup of a short-lived test volume without crashing the host.

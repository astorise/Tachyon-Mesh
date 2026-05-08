# Title: Advanced MCP Tools: Runtime Observability and Agentic Operations

## Problem Statement
While the core Tachyon-MCP server exposes basic status and lockfile reading, it is currently "under-equipped" for autonomous AI operations. AI agents lack access to real-time telemetry (metrics, logs), cannot safely preview changes before applying them, and cannot proactively test resilience. This limits the MCP to a passive indexing role rather than an active control interface.

## Objective
Upgrade `tachyon-mcp` by implementing a suite of advanced read/write tools that empower LLMs to act as autonomous operators:
1. **Runtime Observability:** `tachyon_get_metrics`, `tachyon_tail_logs`, and `tachyon_get_shadow_diffs`.
2. **Safe Operations:** `tachyon_dryrun_manifest` for pre-flight validation without persistence.
3. **Agentic Resilience Testing:** `tachyon_run_chaos_scenario` to allow agents to deliberately inject faults and measure recovery.
# Implementation Tasks

## Phase 1: Wasmtime Engine Update
- [x] `consume_fuel(true)` is already configured in `build_engine` for the
      metered engine; no additional Wasmtime change needed.
- [x] `set_fuel(execution.config.guest_fuel_budget)` is already wired for
      sampled executions via `maybe_set_guest_fuel_budget` /
      `sampled_fuel_consumed`.

## Phase 2: Host Telemetry Hook
- [x] Introduce a new `metering_outbox` redb table. The host now durably
      stages every metering record into the outbox immediately before
      dispatching it via the existing async telemetry export path.
- [x] On successful dispatch the outbox keys are deleted; on failure the
      keys remain so a future retry (or a process restarted from a crash)
      can still drain them. This matches the at-least-once delivery
      semantics the proposal calls out.
- [x] The request critical path is unchanged — the original `mpsc`
      telemetry channel is non-blocking; we only added a durable shadow.

## Phase 3: Metering FaaS Refactoring
- [ ] Switching `system-faas-metering` to consume directly from
      `metering_outbox` (instead of the synchronous `/system/metering`
      route) is left as a small Session B follow-up. The outbox is now
      authoritative and forward-compatible.

## Phase 4: Validation
- [x] Existing telemetry-export unit tests cover the synchronous emit path.
- [x] The outbox helpers (`append_outbox`, `peek_outbox`) carry their own
      tests via the wider store test surface.
- [ ] Manual: deploy a CPU-intensive module, query `metering_outbox` after
      a sweep, confirm fuel sums match. Left for the homelab smoke test.

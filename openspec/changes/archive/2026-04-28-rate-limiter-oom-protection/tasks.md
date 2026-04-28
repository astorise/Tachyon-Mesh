# Implementation Tasks

## Phase 1: Dependencies and Structure
- [x] Open `core-host/Cargo.toml` and ensure a bounded cache library (like `moka`) is available.
- [x] Open `core-host/src/rate_limit.rs`.
- [x] Identify the unbounded data structure currently used for IP tracking.
- [x] Replace it with a bounded LRU Cache configured with a maximum capacity of `100,000` items and a Time-To-Idle TTL.

## Phase 2: Logic Integration
- [x] Update the IP tracking logic (incrementing hit counters) to work with the new Cache API.
- [x] Ensure that updating an existing entry correctly refreshes its LRU position (so active attackers aren't accidentally evicted and forgiven).

## Phase 3: Validation
- [x] **Test Constant Memory:** Write a unit/integration test in `rate_limit.rs` that simulates hitting the rate limiter with `200,000` unique IP addresses.
- [x] Verify that after the simulation, the cache size is exactly `100,000`, proving that the oldest entries were safely evicted and memory did not grow unbounded.

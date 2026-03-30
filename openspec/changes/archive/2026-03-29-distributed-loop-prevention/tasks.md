# Tasks: Change 017 Implementation

- [x] Add an inbound hop-limit middleware in `core-host` that defaults missing or invalid `X-Tachyon-Hop-Limit` values to `10`, stores the parsed value in request extensions, and rejects exhausted requests with HTTP `508 Loop Detected`.
- [x] Propagate a decremented hop limit on host-driven mesh fetches, including relative mesh route targets, so recursive service chains stop instead of running indefinitely.
- [x] Add the `guest-loop` legacy guest plus sealed runtime/build artifacts and regression coverage proving `/api/guest-loop` returns HTTP `508` when it loops back into itself.

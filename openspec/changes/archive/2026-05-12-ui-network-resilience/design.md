# Design: Bounded Network Reconnection & Fallback State

## Approach

Three coordinated changes with no additional dependencies. The existing Zustand store is the single source of truth; the network layer updates it; the UI component subscribes.

### 1. Store extension (`connectionStore.ts`)

Two new fields — `attempt: number` and `maxAttempts: number` — carry retry telemetry without breaking existing consumers of `retryCount`/`incrementRetry`/`resetRetry` (used by `applyAndSeal` path).

`setReconnectionAttempt(attempt, max)` is the write path called by the network layer on each probe failure. It atomically sets both fields and transitions `status` to `"reconnecting"`.

`manualRetry()` resets `attempt` to 0 and dispatches the `"network:manual-retry"` custom event. Using a DOM event avoids a circular import between `connectionStore` and `network.ts`.

### 2. Bounded reconnect loop (`network.ts`)

`MAX_RETRIES = 5` is a module-level constant. The previous unbounded `while` loop is replaced with a `for (attempt = 1; attempt <= MAX_RETRIES; attempt++)` loop:

- Each iteration: calls `setReconnectionAttempt(attempt, MAX_RETRIES)` → probes `get_engine_status` → sleeps with exponential backoff `min(1000 × 2^(attempt-1), 30 000)`.
- On success: `resetRetry()` + `setStatus("connected")`, loop exits.
- After 5 failures: `setStatus("disconnected")` with `attempt` left at 5 so the UI can distinguish the terminal state from a transient one. `reconnectLoop` is cleared so a manual retry can start a fresh cycle.

A `"network:manual-retry"` event listener (registered at module load) sets `reconnectLoop = null` and calls `startReconnectLoop()`, enabling the store-triggered retry without creating a circular dependency.

### 3. NetworkStatus component (`NetworkStatus.ts`)

Switches from `innerHTML` template to explicit DOM node construction (consistent with CSP hardening from the prior change). Subscribes to `{ status, attempt, maxAttempts }`.

Label logic:
- `reconnecting` → `"Reconnecting (N/5)"`
- `disconnected` + `attempt >= maxAttempts > 0` → `"Cluster unreachable"`
- `disconnected` otherwise → `"Offline"`
- `connected` → `"Connected"`

A Retry button is shown whenever `status === "disconnected"` and calls `connectionStore.getState().manualRetry()`.

## Trade-offs

| Decision | Chosen | Rejected | Reason |
|---|---|---|---|
| Retry cap location | `network.ts` constant | Store field | Network policy belongs in the network layer; store holds observable state only |
| Manual retry coupling | DOM custom event | Direct import of network.ts in store | Avoids circular dependency; decoupled by design |
| Terminal state detection | `attempt >= maxAttempts` | Separate `exhausted` boolean | Fewer fields; the invariant is self-documenting from two existing values |
| DOM construction | `createElement` nodes | `innerHTML` template | Consistent with CSP `script-src 'self'` hardening; no escaping risk |

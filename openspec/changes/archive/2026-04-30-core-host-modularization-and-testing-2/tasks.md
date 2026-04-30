# Implementation Plan: Surgical Monolith Extraction

**🛑 CRITICAL SYSTEM INSTRUCTIONS FOR THE AI AGENT (CODEX) 🛑**
1. **NO PLACEHOLDERS:** You are strictly forbidden from using comments like `// ... rest of code ...` or creating empty modules. You MUST physically cut the code from `main.rs` and paste it into the new files.
2. **ITERATIVE COMPILATION:** After EVERY single phase below, you MUST ensure all module imports (`use crate::...`) are updated. Do not proceed to the next phase if the code is severely broken.
3. **PRESERVE LOGIC:** Do not rewrite the business logic during extraction. Move it exactly as it is, only changing visibility (`pub`, `pub(crate)`) and imports.

---

## Phase 1: Global State Extraction (`state.rs`)
The goal is to isolate the data structures that hold the application together.
- [x] 1. Create the file `core-host/src/state.rs`.
- [x] 2. In `main.rs`, locate `struct AppState`, `struct RuntimeState`, `struct FaaSConfig`, and any associated `impl` blocks.
- [x] 3. **CUT** these structures from `main.rs` and **PASTE** them into `state.rs`.
- [x] 4. Make these structs and their fields `pub`.
- [x] 5. In `main.rs`, add `pub mod state;` at the top.
- [x] 6. In `main.rs` (and anywhere else needed), add `use crate::state::{AppState, RuntimeState, FaaSConfig};`. Fix compilation errors related to state.

## Phase 2: Telemetry & Observability (`telemetry/mod.rs`)
- [x] 1. Create directory `core-host/src/telemetry/` and file `mod.rs`.
- [x] 2. Locate all logging initialization functions (e.g., `init_logging`, tracing subscriber setups) and OpenTelemetry metric definitions in `main.rs`.
- [x] 3. **CUT** and **PASTE** them into `telemetry/mod.rs`.
- [x] 4. In `main.rs`, add `pub mod telemetry;`.
- [x] 5. Fix imports in `main.rs` (e.g., `use crate::telemetry::init_logging;`).

## Phase 3: Identity & Security (`identity/mod.rs`)
- [x] 1. Create directory `core-host/src/identity/` and file `mod.rs`.
- [x] 2. Locate `struct CallerIdentityClaims`, JWT parsing logic, mTLS SAN extraction functions, and RBAC evaluation logic in `main.rs` (or `auth.rs` if it exists but is poorly integrated).
- [x] 3. **CUT** and **PASTE** this logic into `identity/mod.rs`.
- [x] 4. Ensure all crypto dependencies (`jsonwebtoken`, `rustls` imports related to certs) are moved here.
- [x] 5. In `main.rs`, add `pub mod identity;` and fix imports.

## Phase 4: Wasm Runtime & FaaS Pooling (`runtime/mod.rs`)
This is the most critical extraction. Be meticulous.
- [x] 1. Create directory `core-host/src/runtime/` and file `mod.rs`.
- [x] 2. Locate the `wasmtime::Engine` initialization, the FaaS component loading logic, and the `Component::deserialize` logic (the Cwasm cache).
- [x] 3. Locate the Wasm instance pooling logic (e.g., `struct WasmPool`, hibernation logic).
- [x] 4. **CUT** and **PASTE** all of this into `runtime/mod.rs`.
- [x] 5. In `main.rs`, add `pub mod runtime;`.
- [x] 6. Fix imports. *Note: `runtime/mod.rs` will likely need `use crate::state::RuntimeState;`.*

## Phase 5: Network, HTTP/3, and Routing (`network/mod.rs`)
- [x] 1. Create directory `core-host/src/network/` and file `mod.rs`.
- [x] 2. Locate the core HTTP/3 (QUIC) listener loop, the TCP/UDP Layer 4 routing logic, and the HTTP request dispatcher/router.
- [x] 3. **CUT** and **PASTE** them into `network/mod.rs`.
- [x] 4. In `main.rs`, add `pub mod network;`.
- [x] 5. Fix imports. The network module will need access to `crate::runtime::*` to pass HTTP requests to the Wasm modules.

## Phase 6: The `main.rs` Cleanup & Panic Hunt
At this point, `main.rs` should ideally contain ONLY the `main()` function, the CLI `clap::Parser` struct, and top-level orchestration.
- [x] 1. Review `main.rs`. Move any lingering helper functions to the appropriate modules.
- [x] 2. **HUNT THE PANICS:** Execute a global search in `core-host/src/` for the exact string `panic!`.
- [x] 3. There are exactly 11 `panic!` macros hidden in production logic. 
- [x] 4. **REPLACE THEM:** Convert every single `panic!(...)` into `return Err(CoreError::Internal(...).into());` or equivalent `thiserror` propagation. Do NOT leave a single `panic!` in production code.
- [x] 5. Run `cargo check` and `cargo clippy`. The build must succeed without warnings related to dead code or unresolved imports.

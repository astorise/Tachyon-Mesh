# Design: Module Architecture & State Ownership

## 1. The Entry Point (`core-host/src/main.rs`)
The new `main.rs` should look structurally similar to this:
```rust
mod error;
mod identity;
mod network;
mod runtime;
mod state;
mod telemetry;

use crate::state::AppState;
use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. Parse CLI arguments
    let args = Cli::parse();

    // 2. Initialize Telemetry
    telemetry::init_logging(&args)?;

    // 3. Boot State & Runtime
    let state = AppState::bootstrap(&args).await?;

    // 4. Start Network Listeners (blocks forever)
    network::start_server(state).await?;
    
    Ok(())
}
```

## 2. Dealing with the 11 `panic!` Calls
During the extraction, the agent **must** actively search for `panic!`, `unwrap()`, or `expect()` within the extracted blocks.
- **Rule:** If the extraction encounters an unwrap related to startup configuration (e.g., binding to a port), it must be converted to an initialization `Result` returned to `main`.
- **Rule:** If it encounters an unwrap in the request path, it must be converted to a `TachyonError` (or `CoreError`) and handled by returning an HTTP 500.
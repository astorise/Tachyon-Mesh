# Tasks: Change 034 Implementation

**Agent Instruction:** Read the proposal.md and specs.md. Implement the one-shot command execution mode and the GC System FaaS without using nested code blocks in your outputs.

## [TASK-1] Host CLI 'Run' Subcommand
- [x] In core-host/src/main.rs, use the clap crate to distinguish between two modes: serve (default, starts Axum) and run (batch execution).
- [x] The run command must take a --target string argument.
- [x] In the run logic, bypass the Axum router. Load the target from the TargetRegistry.
- [x] Configure Wasmtime to expect a Command component using wasmtime::component::Command::instantiate_async.
- [x] Call the method to execute the run export.
- [x] Await the result. If successful, use std::process::exit(0). If it returns an error, log it and use std::process::exit(1).

## [TASK-2] Create the GC System FaaS
- [x] Create a new WASM component project named system-faas-gc. Ensure it is compiled as a Command (wasm32-wasi, not a reactor).
- [x] Read the TTL_SECONDS environment variable using std::env::var. Parse it into an integer.
- [x] Read the TARGET_DIR environment variable (e.g., /cache).
- [x] Write the recursive directory sweeping logic using standard std::fs operations.
- [x] Delete files where the modification time exceeds the TTL compared to the current system time.

## Validation Step
- [x] Build the system-faas-gc.wasm module.
- [x] Update integrity.lock to define the target gc-job, mounting a local /tmp/test-cache directory to /cache, and setting the ENV vars.
- [x] Create some dummy files in /tmp/test-cache and alter their modification times to simulate old files.
- [x] Run the core-host using the new CLI command targeting the gc-job.
- [x] Verify the process runs, prints its deletion logs, and immediately exits back to the terminal prompt with Exit Code 0.
- [x] Verify the old files are deleted from your local filesystem.

# Implementation Tasks

## Phase 1: Configuration Schema
- [x] Add the `requires_tee` boolean to the `integrity.lock` JSON schema and Rust structs.
- [x] Update Tachyon Studio (UI) to display a "Confidential Computing" toggle for deployed functions.

## Phase 2: Engine Branching
- [x] In `core-host`, locate the execution dispatcher.
- [x] Implement the branching logic to route `requires_tee: true` requests away from the standard Instance Pool.

## Phase 3: TEE Runtime Integration
- [x] Bootstrap `systems/system-faas-tee-runtime`.
- [x] Integrate a TEE abstraction layer (such as the `enarx` crate or specialized Wasmtime SGX patches) to handle the actual hardware enclave instantiation.
- [x] Implement a fallback mechanism: if the physical Edge node does not have an SGX/SEV capable CPU, the deployment of a `requires_tee` module MUST fail with a clear hardware incompatibility error, rather than silently falling back to insecure RAM.

## Phase 4: Validation
- [x] **Hardware Test:** Deploy a standard FaaS and a TEE FaaS on an Intel SGX-enabled machine (or Azure Confidential VM).
- [x] Verify that the standard FaaS executes in < 1ms, while the TEE FaaS takes longer but successfully executes.
- [x] **Security Verification:** (Advanced) Attempt to attach a debugger (`gdb`) to the host process during execution. Verify the standard module's memory can be inspected, but the TEE module's memory is inaccessible.

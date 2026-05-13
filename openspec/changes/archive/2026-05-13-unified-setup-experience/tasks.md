# Implementation Tasks

- [x] **Task 1: Script Creation**
  - Create `scripts/setup.sh` based on the specification.
  - Make the script executable (`chmod +x scripts/setup.sh`).

- [x] **Task 2: Prerequisite Checks**
  - Ensure the script fails gracefully with helpful error messages if Rust, `cargo`, or `npm` are missing.

- [x] **Task 3: Documentation Update**
  - Update the `README.md` "Quick Start" section to point users immediately to `./scripts/setup.sh` as step 1.

- [x] **Task 4: Cross-Platform (Optional but recommended)**
  - If Windows developers are a target, consider a parallel `scripts/setup.ps1` or shifting the orchestration logic into a cross-platform tool like `just` (Justfile) or a simple Rust CLI orchestrator.

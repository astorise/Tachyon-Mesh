## 1. Change Repair

- [x] 1.1 Replace the invalid flat `specs.md` artifact with capability deltas under `specs/`.
- [x] 1.2 Rewrite the proposal, design, and tasks artifacts so the change matches the current sealed-route architecture.

## 2. Implementation

- [x] 2.1 Extend `tachyon-cli` route metadata so generated manifests can seal `min_instances` and `max_concurrency`, with backward-compatible defaults.
- [x] 2.2 Add an optional CLI scaling override input and keep route normalization, role handling, and secret grants working together.
- [x] 2.3 Extend `core-host` route validation to accept the new sealed fields, reject invalid concurrency values, and build a shared semaphore map per route.
- [x] 2.4 Switch the shared Wasmtime engine to the pooling allocator while preserving fuel metering and component-model support.
- [x] 2.5 Enforce per-route concurrency in `faas_handler`, returning HTTP 503 after a five-second semaphore wait timeout.
- [x] 2.6 Regenerate the checked-in `integrity.lock` manifest with the updated CLI payload shape.

## 3. Verification

- [x] 3.1 Add or update Rust tests covering route scaling parsing, integrity defaults, and route concurrency exhaustion behavior.
- [x] 3.2 Run `openspec validate --all`, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace`, `cargo build -p core-host --release`, and `cargo build -p tachyon-cli --release`.

## 4. Delivery

- [x] 4.1 Commit and push the scoped change on the active branch.
- [x] 4.2 Iterate on GitHub Actions failures until the branch is green.
- [x] 4.3 Archive `instance-pooling-concurrency` with spec sync once the branch head is green.

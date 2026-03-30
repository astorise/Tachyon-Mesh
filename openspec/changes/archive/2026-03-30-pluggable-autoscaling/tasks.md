# Tasks: Change 021 Implementation

- [x] 1.1 Convert the change to the spec-driven layout by adding `design.md` and
  delta specs for `faas-observability` plus `http-routing`.
- [x] 2.1 Extend `wit/tachyon.wit` with `scaling-metrics` and the
  `background-system-faas` world.
- [x] 2.2 Track pending route waiters in `core-host` and expose
  `get-pending-queue-size` to privileged System FaaS guests.
- [x] 2.3 Start persistent five-second background workers only for sealed `system`
  components that implement `on-tick`.
- [x] 3.1 Add the `system-faas-keda` component guest and render Prometheus queue
  depth for `/api/guest-call-legacy`.
- [x] 3.2 Add the `system-faas-k8s-scaler` background guest with threshold-based
  mock Kubernetes `PATCH` requests and a thirty-second cooldown.
- [x] 4.1 Update workspace packaging and CI so the new System FaaS artifacts are
  built and copied into the runtime image.
- [x] 4.2 Add or update host tests covering the scaling metrics route and the
  background scaler cooldown behavior.
- [x] 5.1 Document how to seal `/metrics/scaling` and `/system/k8s-scaler`, then
  verify with `cargo fmt`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
  `cargo test --workspace`, and `openspec validate --all`.

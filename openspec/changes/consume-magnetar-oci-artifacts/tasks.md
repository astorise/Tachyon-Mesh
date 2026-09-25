**Do not start section 1 until a real Magnetar Component/Kernel artifact has
been published (a GitHub Release or GHCR tag Magnetar itself produced via
`release-publication.yml`/`component-artifact-distribution.yml`) — see
proposal.md's "Why". Confirm one exists and note its locator/digest here
before beginning implementation.**

## 0. Precondition

- [ ] 0.1 A real Magnetar-published artifact exists and its locator + digest
      are recorded here: `locator = ???`, `expected_digest = sha256:???`.

## 1. `ArtifactReference` and resolver skeleton

- [ ] 1.1 `ArtifactReference { locator, expected_digest }` type.
- [ ] 1.2 New optional Cargo feature (e.g. `oci-artifacts`) gating the OCI
      client dependency — `ai-inference` alone must not pull it in.
- [ ] 1.3 `ArtifactResolver`: registry auth, manifest/layer pull, digest
      verification before any bytes are exposed downstream.

## 2. Local content-addressed cache

- [ ] 2.1 Atomic staging into `sha256/<digest>/` (no partial writes visible).
- [ ] 2.2 Cache hit path takes no network access.
- [ ] 2.3 Garbage collection for unreferenced digests.
- [ ] 2.4 A corrupted cache entry is detected and rejected before load, not
      after a failed/garbled invocation.

## 3. Wiring

- [ ] 3.1 Resolver output feeds `MagnetarRuntime::try_load` as a local path
      — confirm `MagnetarRuntime`'s own surface is unchanged (no new public
      methods) by re-running `scripts/validate_ai_component_boundary.sh`.
- [ ] 3.2 A binding using an `oci://` locator behaves identically, end to
      end, to one given the same bytes through a local directory — same
      trust checks, same invocation path, no special-cased behavior.

## 4. Tests (need the real artifact from task 0.1)

- [ ] 4.1 Pull by digest succeeds.
- [ ] 4.2 A tag currently resolving to the expected digest succeeds.
- [ ] 4.3 A tag repointed to an unexpected digest is rejected.
- [ ] 4.4 Publisher signature valid (if `integrate-magnetar-publisher-trust`
      has landed) → succeeds; revoked key → rejected.
- [ ] 4.5 Cache hit path: second load of the same digest touches no network.
- [ ] 4.6 The same digest supplied through a local directory (no OCI
      involved) produces identical behavior — transport is provably
      irrelevant to trust/load outcome.
- [ ] 4.7 A corrupted cache entry is caught before execution, not during it.

## 5. Closure

- [ ] 5.1 `cargo test -p core-host --features ai-inference,oci-artifacts`
      green, including section 4.
- [ ] 5.2 `cargo clippy --workspace --all-targets --features core-host/ai-inference,core-host/legacy-wasi-nn -- -D warnings -D clippy::unwrap_used` clean (and with the new feature added to that combo).
- [ ] 5.3 `bash scripts/validate_ai_component_boundary.sh` clean — confirms
      `MagnetarRuntime` gained no OCI/registry surface (TACH-10's explicit
      "what the bridge must not become" guardrail).
- [ ] 5.4 This closes TACH-10 and TACH-13. Archive this change once done.

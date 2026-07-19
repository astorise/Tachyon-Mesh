## 1. Actionable rejection errors

- [x] 1.1 In `core-host/src/host_core/integrity_config.rs::validate_require_scopes`, extend the "missing `scopes` block" error to name `tachyon_suggest_scopes` as the remediation tool, keeping the existing `scopes`/`require_scopes` substrings intact.
- [x] 1.2 Extend the "resolves to allow-all" error the same way, keeping the existing `allow-all`/`scopes` substrings intact.
- [x] 1.3 Update `core-host/src/host_core/tests/config_validation.rs` (`require_scopes_flag_rejects_missing_scopes_block`, `require_scopes_flag_rejects_allow_all_scopes`) to assert the new error text names `tachyon_suggest_scopes`.
- [x] 1.4 Run `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets --features core-host/ai-inference -- -D warnings -D clippy::unwrap_used` for the touched crate.

## 2. Legacy opt-out documentation

- [x] 2.1 Add a note to `docs/faas-import-scoping.md`'s "Phase 4 — flip the default" section documenting that `require_scopes: false` remains a supported, permanent explicit setting after the fleet default changes — not a deprecated flag.
- [x] 2.2 Cross-link the new note to the rollback note already present in that section so operators find both together.

## 3. Deferred follow-ups (outside this change, tracked by issue #311)

- Run the 2+ week observation window against the D1 criteria in `design.md`: `faas_scopes_allow_all_total` flat fleet-wide, `tachyon_get_scope_denials` reporting `allow_all: false` for every route, no unexplained `require_scopes: false`.
- Migrate `/metrics` and `/system/logger` in the reference `integrity.lock` off allow-all after link-time verification and the addition of re-signing tooling analogous to `scripts/seal-guest-openai.js`.
- After the observation window closes successfully, open a separate Phase 4 OpenSpec change that flips the compiled-in `require_scopes` default to `true` and cites the collected evidence.

## 4. Pre-merge validation (for tasks 1–2 only)

- [x] 4.1 `cargo test -p core-host` covering the modified `config_validation.rs` tests.
- [x] 4.2 Confirm no other test asserts the exact prior wording of the two `validate_require_scopes` error strings (grep for the literal phrases before changing them).

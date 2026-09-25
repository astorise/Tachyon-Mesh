## Summary

<!-- What changed and why. Link the issue this closes, if any: `Fixes #N` /
     `Closes #N` — see CONTRIBUTING.md's branch and issue discipline. -->

## Validation

<!-- What you ran locally before opening this PR. At minimum, for Rust
     changes, the fast quality gate from CONTRIBUTING.md:

     cargo fmt --all -- --check
     cargo clippy --workspace --all-targets --features core-host/ai-inference,core-host/legacy-wasi-nn -- -D warnings -D clippy::unwrap_used
     for feature in http3 legacy-wasi-nn mtls rate-limit resiliency s3-persistence secrets-vault websockets; do
       cargo check -p core-host --features "$feature"
     done

     Plus the smallest relevant `cargo check`/test for any feature-gated code
     you touched, and `npm test` / `npx tsc --noEmit` for tachyon-ui changes. -->

## Risk / self-review notes

<!-- If this touches more than a small, focused set of files, or you're the
     sole reviewer on this change, note the risk areas here: what could break,
     what you didn't test, and why you're confident regardless. -->

## Checklist

- [ ] This PR is up to date with `main`.
- [ ] `quality`, `cuda-quality`, `security-audit`, and `build-guests` are
      expected to pass (or I've explained why one legitimately won't).
- [ ] Any behavior change is reflected in `CHANGELOG.md` and, if it affects a
      documented capability, the relevant `docs/` page or `openspec/specs/`
      requirement.
- [ ] No secrets, tokens, or credentials in this diff.

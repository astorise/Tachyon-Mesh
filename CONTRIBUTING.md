# Contributing

## Branch and issue discipline

`main` is a protected branch. Do not push implementation work directly to it.
Every change must land through a pull request targeting `main`.

Before a pull request can merge, it must be up to date with `main` and pass the
required GitHub Actions status checks:

- `quality`
- `cuda-quality`
- `security-audit`
- `build-guests`

Close issues only when the fix has landed on `main`. Prefer `Fixes #N` or
`Closes #N` in the pull request body so GitHub closes the issue automatically
when the PR is merged. Do not close an issue while its fix exists only on an
unmerged branch.

For large batches, especially changes touching more than a small focused set of
files, request review before merge. If the work is solo-maintained, leave a
short self-review note in the PR covering the risk areas and validation run.

## Local validation

Run the fast quality gate before opening or updating a PR when the change
touches Rust code:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --features core-host/ai-inference -- -D warnings -D clippy::unwrap_used
```

For feature-gated code, also run the smallest relevant `cargo check` or test
command matching the feature you changed.

# Design: GitHub Actions Workflow Hardening

## 1. Feature Matrix Definition (`.github/workflows/ci.yml`)
Update the test job to use a build matrix to cover the "zero-cost" optional features:
```yaml
strategy:
  matrix:
    features: 
      - "" # Default
      - "--all-features"
      - "--no-default-features"
      - "--features ai-inference,http3-quic"
      - "--features chaos,canary"
```

## 2. Security Job Addition
Create a dedicated `Security Audit` job in the workflow:
```yaml
security_audit:
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
    - uses: rustsec/audit-check@v1.4.1 # cargo audit
    - uses: EmbarkStudios/cargo-deny-action@v1 # cargo deny
```

## 3. Miri Validation for Unsafe Code
To avoid long CI times, Miri will be scoped only to the `core-host/src/runtime` (or wherever `unsafe` resides):
```yaml
miri_test:
  runs-on: ubuntu-latest
  steps:
    - run: rustup toolchain install nightly --component miri
    - run: cargo +nightly miri test --manifest-path core-host/Cargo.toml -- lib::runtime::cwasm_cache
```
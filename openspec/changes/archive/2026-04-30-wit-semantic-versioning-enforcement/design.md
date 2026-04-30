# Design: WIT Package Syntax

## 1. Standardized Header
Every WIT file in `sdk/wit/` must start with a semantic version:
```wit
package tachyon:mesh@1.0.0;

interface request-handler {
    // ...
}
```

## 2. Compatibility Matrix Workflow (`.github/workflows/wit-compat.yml`)
```yaml
name: WIT Compatibility Check
on: [pull_request]
jobs:
  check-compat:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
      - name: Install wasm-tools
        run: cargo install wasm-tools
      - name: Check Backward Compatibility
        run: |
          # Compare current PR WITs with origin/main
          git checkout origin/main -- sdk/wit/tachyon.wit
          mv sdk/wit/tachyon.wit old_tachyon.wit
          git checkout - -- sdk/wit/tachyon.wit
          
          wasm-tools component wit old_tachyon.wit > old.wasm
          wasm-tools component wit sdk/wit/tachyon.wit > new.wasm
          
          wasm-tools component check old.wasm new.wasm
```
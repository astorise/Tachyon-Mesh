# Design: Automated SDK Pipeline

## 1. The Single Source of Truth
We consolidate all isolated `.wit` files into a standardized, versioned directory: `sdk/wit/tachyon.wit`. This file acts as the ultimate API contract between the host and the tenant FaaS.

## 2. CI/CD Generation Pipeline (`.github/workflows/publish-sdks.yml`)
Instead of committing generated code, the CI handles generation and publishing upon a new GitHub Release or Tag (e.g., `v1.2.0`).

### Step A: Rust (Crates.io)
```bash
cargo install wit-bindgen-cli
wit-bindgen rust ./sdk/wit --out-dir ./sdk/rust/src
cd ./sdk/rust && cargo publish --token ${{ secrets.CARGO_REGISTRY_TOKEN }}
```

### Step B: JavaScript/TypeScript (NPM)
```bash
npm install -g @bytecodealliance/jco
jco transpile ./sdk/wit -o ./sdk/js/src --name tachyon-sdk
cd ./sdk/js && npm publish --access public --token ${{ secrets.NPM_TOKEN }}
```

### Step C: Python (PyPI)
Using `componentize-py` or `wit-bindgen`:
```bash
wit-bindgen python ./sdk/wit --out-dir ./sdk/python/tachyon_sdk
cd ./sdk/python && poetry publish --build --username __token__ --password ${{ secrets.PYPI_TOKEN }}
```

## 3. Breaking Change Detection
In our standard `ci.yml` (run on every PR), we will add a step that uses `wit-bindgen --check-compat` to compare the modified `tachyon.wit` against the `main` branch version. If a PR breaks backward compatibility of the API contract, the CI fails immediately.
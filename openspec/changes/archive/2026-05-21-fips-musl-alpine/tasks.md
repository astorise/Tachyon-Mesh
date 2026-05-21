## 1. Dockerfile.fips

- [x] 1.1 Create `Dockerfile.fips` with `wasm-builder` (Ubuntu) and `fips-builder` (rust:alpine) stages
- [x] 1.2 Install Alpine FIPS build deps in fips-builder: cmake, nasm, go, perl, linux-headers
- [x] 1.3 Compile `core-host --no-default-features --features fips` in fips-builder stage
- [x] 1.4 Add `FROM scratch` runtime stage copying core-host binary and WASM modules
- [x] 1.5 Verify final image size is under 35 MB

## 2. CI — fips-tests job

- [x] 2.1 Add `fips-tests` job to `.github/workflows/ci.yml`
- [x] 2.2 Install cmake, nasm, protobuf-compiler in fips-tests job
- [x] 2.3 Run `cargo test -p core-host --features fips` in fips-tests job
- [x] 2.4 Run `cargo build -p core-host --release --features fips` in fips-tests job
- [x] 2.5 Upload FIPS release binary as CI artifact `core-host-linux-x86_64-fips-<sha>`

## 3. CI — feature-matrix-tests job

- [x] 3.1 Add `feature-matrix-tests` job with strategy matrix of 5 feature combinations
- [x] 3.2 Install FIPS build deps (cmake, nasm, protobuf-compiler) for `--all-features` entry
- [x] 3.3 Run `cargo test -p core-host <features>` and release build per matrix entry
- [x] 3.4 Upload per-combination binary artifact with descriptive label

## 4. CI — Docker publish matrix

- [x] 4.1 Add `-fips` matrix entry to `publish-docker-images` using `Dockerfile.fips`
- [x] 4.2 Verify `-fips` image is tagged `latest-fips` and `sha-<sha>-fips` on main push
- [x] 4.3 Add `-http3` and `-security` Docker variants for full feature coverage

## 5. CI — rust-ci protobuf dependency

- [x] 5.1 Add `protobuf-compiler` to Linux system deps in `rust-ci` job (needed for `--all-features` / prost)
- [x] 5.2 Add `protobuf-compiler` to `feature-matrix-tests` `--all-features` conditional install

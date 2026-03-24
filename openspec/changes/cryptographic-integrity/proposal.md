# Proposal: Change 005 - Cryptographic Integrity & Validation

## Context
In a standard FaaS environment, configuration (like routing rules or resource limits) is loaded at runtime via environment variables or YAML files. This makes the system vulnerable to post-deployment tampering and configuration drift. To eliminate this, we want to cryptographically seal the configuration using Ed25519 signatures. 

## Objective
Implement a build-time and run-time integrity check. We will introduce a small CLI tool (`cli-signer`) that generates an Ed25519 key pair, signs a predefined JSON configuration, and writes an `integrity.lock` file. The `core-host` will embed the public key and expected signature at compile-time (via `build.rs`) and verify the actual runtime configuration against this signature upon startup.

## Scope
- Add `ed25519-dalek`, `sha2`, and `serde` dependencies.
- Create a new workspace binary `cli-signer` to generate the `integrity.lock` file.
- Add a `build.rs` script to `core-host` to read the `integrity.lock` and pass its contents to the compiler.
- Update `core-host/src/main.rs` to compute the hash of its startup configuration and verify the Ed25519 signature before binding the HTTP server.

## Out of Scope
- The full Tauri graphical interface (this CLI is the foundational logic).
- Asymmetric encryption of runtime secrets (we focus on configuration integrity first).

## Success Metrics
- Running `cli-signer` produces a valid `integrity.lock` JSON file.
- Compiling `core-host` successfully bakes the public key and signature into the binary.
- If the configuration hash matches the signature, the Host starts and serves HTTP requests.
- If the configuration is tampered with, the Host panics with an "Integrity Validation Failed" error immediately on startup.
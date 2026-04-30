# Proposal: Official Polyglot FaaS SDK

## Context
While `core-host` supports multiple languages via WebAssembly and the Component Model (WIT), developers currently have to manually generate bindings from our `.wit` files or rely on scattered examples (`guest-go`, `guest-js`). To drive Enterprise adoption, we need official, versioned, and documented SDKs distributed through standard package managers.

## Proposed Solution
Establish a single source of truth (`tachyon.wit`) and automatically compile, package, and publish idiomatic SDKs for the top enterprise languages.
- **Rust:** Publish `tachyon-faas-sdk` on `crates.io`.
- **JavaScript/TypeScript:** Publish `@tachyon-mesh/sdk` on `npm` (via `jco`).
- **Python:** Publish `tachyon-sdk` on `PyPI`.
- **Go:** Publish `github.com/tachyon-mesh/sdk-go`.

## Objectives
- Reduce developer onboarding time from "hours figuring out WIT" to "seconds running npm install".
- Ensure backward compatibility through strict semantic versioning of the SDKs.
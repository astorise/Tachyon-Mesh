# Proposal: Opt-In Confidential Computing (TEE)

## Context
Our Transparent Data Encryption (TDE) protects data at rest. Our mTLS/Noise overlay protects data in transit. However, a critical vulnerability remains for "data in use": if an attacker gains root access to the physical Edge node or compromises the Kubernetes hypervisor, they can perform a memory dump to extract plaintext AI prompts, API keys, or healthcare data directly from the host's RAM.

## Proposed Solution
We will implement an **Opt-In Confidential Computing layer**:
1. **Granular Configuration:** The `integrity.lock` manifest will allow developers to flag specific, highly sensitive FaaS modules with `requires_tee: true`.
2. **Hybrid Execution Engine:** By default, `core-host` uses our pooled Wasmtime engine for maximum speed. If `requires_tee` is detected, the host bypasses the standard pool.
3. **Hardware Enclave Delegation:** The host delegates the execution of that specific Wasm module to a TEE-compatible backend (e.g., integrating with the Enarx framework, WasmEdge SGX, or AWS Nitro Enclaves). The code and data are executed inside a hardware-encrypted memory space.
4. **Strict Isolation:** The enclave is completely opaque to the host OS. Even a root user running `gdb` or `dd` on `/dev/mem` will only see encrypted garbage.

## Objectives
- Provide military-grade security for processing PII (Personally Identifiable Information) and confidential AI models at the Edge.
- Maintain zero latency overhead for 99% of non-sensitive FaaS traffic by keeping the feature strictly opt-in.
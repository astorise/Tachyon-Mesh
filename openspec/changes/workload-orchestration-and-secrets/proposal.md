# Proposal: Workload Orchestration & Secrets Configuration Schema

## Context
Tachyon Mesh supports executing functions via the standard Component Model (`faas-wasm`), highly isolated microVMs (`smolvm`), and proxying to external generic containers (`legacy-container`). These workloads require environment variables and sensitive secrets (API keys, DB credentials) to function.

## Problem
Storing secrets in plaintext within GitOps YAML files violates security policies. Furthermore, there is currently no unified declarative API for Tachyon-UI to define a workload's execution runtime (Wasm vs. SmolVM vs. Legacy) alongside its environment context.

## Solution
Introduce the `config-workloads.wit` schema to provide:
1. **Runtime Extensibility**: Declaratively assign workloads to `faas_wasm`, `smolvm_microvm`, or `legacy_container` runtimes.
2. **Environment Variables**: Standard key-value injection for non-sensitive data.
3. **Secret References**: A mechanism to reference secrets by ID. The actual secret is resolved at execution time by `system-faas-tde` (Transparent Data Encryption) or an external Vault, ensuring the GitOps repository remains 100% plaintext-free of sensitive material.
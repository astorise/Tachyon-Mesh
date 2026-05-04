# Proposal: Storage & State Configuration Schema

## Context
WebAssembly components in Tachyon Mesh run in a strictly isolated sandbox. To perform meaningful Edge workloads (caching, database proxies, AI RAG), they require state. Tachyon provides state via WASI persistent volumes, a native KV store (Turboquant), and S3 blob storage proxies.

## Problem
Currently, mounting a volume or targeting an S3 bucket requires static configuration at the host level or hardcoding it inside the Wasm module. This breaks environment portability (Dev -> Staging -> Prod) and prevents Tachyon-UI from managing data-plane storage dynamically.

## Solution
Introduce the `config-storage.wit` schema to the Configuration API. This allows the GitOps broker to declaratively bind virtual guest paths to physical host paths, define logical S3 backend targets, and provision KV partitions.
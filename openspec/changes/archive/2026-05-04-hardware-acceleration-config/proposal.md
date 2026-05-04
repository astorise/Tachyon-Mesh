# Proposal: Hardware Acceleration Configuration Schema

## Context
Tachyon Mesh is designed to exploit Edge-Native hardware capabilities, including eBPF/XDP for zero-overhead networking, TPUs/GPUs for AI inference workloads, and Trusted Execution Environments (TEEs) for Confidential Computing.

## Problem
Currently, hardware utilization is either auto-detected with limited control or requires low-level host configuration. Tachyon-UI needs a declarative schema to enforce hardware policies across the fleet, such as mandating TEE execution for sensitive workloads or reserving TPU memory explicitly.

## Solution
Introduce the `config-hardware.wit` schema. This enables GitOps-driven configuration for:
1. **Network Acceleration**: Enable/Disable eBPF XDP modes dynamically.
2. **Compute Coprocessors**: Allocate GPU/TPU/NPU resources to the Wasm execution engine.
3. **Confidential Computing**: Enforce TEE (SGX, SEV, Nitro) enablement and configure remote attestation endpoints.
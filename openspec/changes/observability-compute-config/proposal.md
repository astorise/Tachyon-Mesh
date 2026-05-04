# Proposal: Observability & Compute Configuration Schema

## Context
Tachyon Mesh requires strict control over its resource footprint (Compute) and its telemetry exhaust (Observability). The WebAssembly execution engine (`microvm-runner`) and the OTLP tracing components need declarative rules to operate efficiently at the Edge.

## Problem
Hardcoding logging levels (e.g., debug vs info) or memory limits per Wasm guest leads to either telemetry flooding (OOM/Disk full) or overly restrictive execution environments. Tachyon-UI needs to be able to dynamically tweak sampling rates and RAM quotas without a host reboot.

## Solution
Define the `config-observability.wit` schema to allow real-time tuning of:
1. **Logging & Tracing**: Dynamic log levels, OTLP endpoint configuration, and distributed tracing sampling rates.
2. **Compute Quotas**: Strict CPU/RAM limits bound to specific Target Groups (Wasm components) enforced by the Memory Governor.
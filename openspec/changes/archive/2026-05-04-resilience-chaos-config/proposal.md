# Proposal: Resilience & Chaos Configuration Schema

## Context
Tachyon Mesh provides advanced L7 features such as retries, timeouts, shadow traffic (mirroring), and chaos engineering (fault injection) via its WebAssembly components. The Control Plane needs a declarative schema to manage these policies.

## Problem
Currently, updating a retry policy or starting a chaos experiment requires touching internal component configurations or hardcoding values. This lacks the GitOps traceability required for Enterprise-Grade operations and prevents Tachyon-UI from offering a unified dashboard for system reliability.

## Solution
Define the `config-resilience.wit` contract and its GitOps YAML equivalent. This schema will allow operators to declaratively attach:
1. **Timeouts & Retries**: For standard L7 resilience.
2. **Shadow Traffic**: To mirror a percentage of traffic to a testing target asynchronously.
3. **Chaos Faults**: To inject latency or HTTP aborts for a specific percentage of requests, enabling automated reliability testing in production.
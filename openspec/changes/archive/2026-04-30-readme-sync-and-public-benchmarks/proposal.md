# Proposal: Documentation Sync & Public Benchmarks

## Context
A project's `README.md` is its landing page; inconsistencies immediately degrade trust. Currently, our README lists Phases 1-3 as "In Progress" despite them being fully specified and delivered. Furthermore, the Quick Start guide has confusing PowerShell instructions for manifest generation. 
Most importantly, our core value proposition—"Zero Overhead Service Mesh"—is a bold claim. Without public, reproducible benchmarks comparing us to industry standards (Istio-Ambient, Linkerd2-proxy), enterprise architects will dismiss this claim as marketing fluff.

## Proposed Solution
1. **README Overhaul:** Rewrite the README to accurately reflect the completed architecture (Day-1 to Day-4). Fix the Quick Start to clearly separate "creating `integrity.lock`" from "running the host".
2. **Reproducible Benchmark Harness:** Create a `bench/` directory containing Terraform/Pulumi scripts and `fortio`/`k6` configurations to automatically spin up a test cluster, deploy Tachyon alongside Envoy/Linkerd, and generate performance graphs.
3. **Transparent Metrics Publication:** Publish the results (Latency p50/p90/p99, CPU usage, RSS Memory) directly in the repository and host the raw data on GitHub Pages.

## Objectives
- Build immediate credibility with a flawless, up-to-date README.
- Prove the "100 MB -> near 0" and "sub-millisecond latency" claims with hard, verifiable data.
- Allow any skeptical engineer to clone the repo, run `make bench`, and see the results on their own hardware.
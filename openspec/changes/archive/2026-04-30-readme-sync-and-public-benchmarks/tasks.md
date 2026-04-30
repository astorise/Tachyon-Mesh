# Implementation Tasks

## Phase 1: README Corrections
- [x] Update the Roadmap section in `README.md` to reflect the completed state of Phases 1-3.
- [x] Rewrite the "Quick Start" section to provide accurate, OS-agnostic (Bash/Zsh) commands for generating and using the `integrity.lock`.
- [x] Add a prominent "Performance & Benchmarks" section linking to the new `bench/` folder.

## Phase 2: Building the Benchmark Harness
- [x] Create the `bench/` directory.
- [x] Write a `k3d` or `minikube` bootstrap script to ensure a clean, reproducible testing environment.
- [x] Configure deployment manifests for a neutral test workload (e.g., a simple HTTP echo server) wrapped by Istio, Linkerd, and Tachyon Mesh.

## Phase 3: Execution & Automation
- [x] Write the load-testing script using `fortio` or `k6` to blast the endpoints and capture the latency distribution.
- [x] Implement a script to query `cAdvisor` or `kubectl top` to measure the idle and active memory consumption (RSS) of the proxy proxies.

## Phase 4: Publication
- [x] Document the standardized cloud instance profile required for baseline numbers.
- [x] Format the output into Markdown tables from raw Fortio JSON and commit the report generator.

# Proposal: GitOps Configuration Broker and Multi-Environment API

## Context
Tachyon Mesh requires a resilient, auditable, and zero-downtime control plane to manage configurations dynamically via Tachyon-UI and MCP. Managing configuration synchronously creates tight coupling and risks data-plane paralysis during network outages.

## Problem
Relying on synchronous external databases or direct S3 calls for configuration state introduces latency, breaks Edge-Native offline capabilities, and lacks built-in environment promotion (Dev -> Staging -> Prod) mechanisms.

## Solution
Implement an Event-Driven, GitOps-based configuration system using WebAssembly components. The solution splits responsibilities into two FaaS systems:
1. `system-faas-config-api`: Validates UI/MCP requests and ensures RBAC.
2. `system-faas-gitops-broker`: A pure Rust (`gix`) git client mapped to a local WASI volume (`/var/lib/tachyon/config-store/`) for offline-first resilience.
The broker syncs with S3 as a remote backend. Node environments are mapped directly to Git branches, enabling seamless promotions and instant rollbacks via standard Git operations.
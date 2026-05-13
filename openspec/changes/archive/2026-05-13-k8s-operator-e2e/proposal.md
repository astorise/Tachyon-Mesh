# Proposal: Kubernetes Operator E2E Testing (vcluster)

## Context
We recently introduced a "Zero-Build" deployment path for operators using `manifests/deploy.yaml`. However, we currently lack a CI/CD mechanism to verify that this exact manifest deploys successfully, mounts volumes correctly, and passes readiness probes in a real Kubernetes environment.

## Problem
Without automated K8s testing, a simple typo in `deploy.yaml` (e.g., an invalid API version or a misconfigured port) could be merged to `main` and break the operator onboarding path, leading to immediate user frustration.

## Proposed Solution
Implement a GitHub Actions workflow that:
1. Spins up a lightweight `k3s` cluster using `k3d`.
2. Installs and deploys a `vcluster` (Virtual Cluster) inside it to simulate a locked-down, multi-tenant enterprise environment.
3. Applies our `manifests/deploy.yaml` directly to the `vcluster`.
4. Runs `kubectl wait` to verify that the `core-host` pods reach the `Ready` state.

## Impact
- **Absolute Confidence:** Guarantees that our primary release artifact (`deploy.yaml`) is completely functional on every commit to `main`.
- **Security Validation:** Running in `vcluster` proves our deployment doesn't require cluster-admin privileges, ensuring compatibility with strict security policies.
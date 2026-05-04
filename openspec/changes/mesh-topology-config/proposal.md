# Proposal: Mesh Topology & Enrollment Configuration Schema

## Context
A Tachyon Mesh cluster relies on a P2P overlay network, decentralized state synchronization (Gossip), and secure node onboarding (Device Flow). The control plane requires a unified, declarative way to govern these global network behaviors.

## Problem
Relying on CLI flags or static environment variables for peer discovery and enrollment strategies makes scaling the mesh cumbersome. If the Gossip heartbeat needs to be tuned to prevent split-brain during network degradation, operators cannot currently hot-reload this across the fleet.

## Solution
Introduce the `config-topology.wit` schema. This enables GitOps-driven, hot-reloadable configuration for:
1. **Gossip Protocol**: Heartbeat intervals, failure detection thresholds, and static seed nodes.
2. **Node Enrollment**: Authentication strategies (e.g., OAuth2 Device Flow) and token lifecycles for Zero-Touch Provisioning.
3. **Mesh Overlay**: Global P2P routing preferences and metadata advertisement.
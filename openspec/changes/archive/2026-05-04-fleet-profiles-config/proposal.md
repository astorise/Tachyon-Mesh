# Proposal: Fleet Profiles & Node Selectors Configuration Schema

## Context
Tachyon Mesh targets highly distributed and heterogeneous Edge environments. Nodes are enrolled with specific metadata tags (e.g., `env=prod`, `role=gateway`, `region=eu-west`). 

## Problem
Currently, our GitOps configuration domains (Routing, Security, Resilience, Compute, Storage, Hardware) apply globally or rely on implicit naming conventions. Tachyon-UI requires a declarative way to group nodes into "Fleets" and selectively bind specific configurations to them without broadcasting unnecessary state across the entire Mesh.

## Solution
Introduce the `config-fleet.wit` schema to define `FleetProfile` objects. These profiles act as logical groups using label-matching mechanisms (Node Selectors). By decoupling the "What" (the config payloads) from the "Where" (the Fleet Profiles), operators can achieve precise, targeted rollouts, canary deployments, and efficient gossip routing.
# Proposal: Control Plane RBAC & ACLs Configuration Schema

## Context
The `system-faas-config-api` exposes powerful MCP and REST/gRPC endpoints to mutate the GitOps state of the entire Edge Mesh (Routing, Security, Hardware, LLM Orchestration). 

## Problem
Without a native Authorization scheme for the configuration API itself, any authenticated user via Tachyon-UI could potentially modify cluster-wide topologies or hardware allocations. We need strict Role-Based Access Control (RBAC) combined with granular Access Control Lists (ACLs) to scope permissions not only by Domain (e.g., Routing vs. Hardware), but also by Fleet or Tenant boundaries.

## Solution
Introduce the `config-rbac.wit` schema. This defines `Role` and `RoleBinding` entities for the Control Plane. 
1. **Domain Granularity**: Control CRUD access to specific WIT schemas (e.g., `Allow Update on config-routing`).
2. **Fleet/Tenant Isolation**: Restrict configurations to specific metadata tags (e.g., `Allow Update on config-routing ONLY IF target FleetProfile matches tenant=finance`).
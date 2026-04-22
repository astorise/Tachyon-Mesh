# Proposal: Change 073 - Navigation & Component Routing

## Context
The current `tachyon-ui` is limited to a single Dashboard view. The sidebar menu items are present but non-functional. To make the Studio usable, we must implement a routing mechanism in the TypeScript frontend to switch between different management planes (Mesh, FaaS, Identity, AI).

## Objective
1. Implement a dynamic component switcher in `main.ts`.
2. Create dedicated HTML view templates for: Mesh Topology, FaaS Registry, Identity Management, and AI Model Broker.
3. Bind the sidebar links to the navigation controller.
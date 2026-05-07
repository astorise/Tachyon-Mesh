# Proposal: Global Telemetry Overview Dashboard

## 1. Context
Currently, after a successful IAM login, the `TachyonAppShell` loads, but the main `#router-view` is empty until the user clicks a sidebar link. We need a default landing page that provides an immediate, high-level overview of the mesh's health.

## 2. Solution
Implement `<tachyon-overview-panel>` which inherits from `TachyonConfigDashboard`. 
This panel will display key telemetry metrics: Active Edge Nodes, Global Wasm Instances, and AI/GPU Hardware Utilization. It will use GSAP to animate counters from 0 to their actual values to create a "boot-up" cyber-aesthetic.

## 3. Design System
Maintain the Dark Slate (900/800) and Cyan (400/500) palette. Use grid layouts for metric cards and monospace fonts for numerical data.
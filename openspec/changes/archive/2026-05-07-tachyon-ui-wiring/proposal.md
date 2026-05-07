# Title: UI Wiring, Dynamic Telemetry & IAM Setup

## Problem Statement
The recent migration to native Web Components (Shadow DOM) established a solid architectural foundation but introduced regressions:
1. Missing route mappings in `ComponentRegistry.ts` break navigation to Mesh Topology and Asset Registry.
2. `TachyonOverviewPanel` relies on hardcoded mock data instead of live Rust backend telemetry.
3. The IAM component (`TachyonIAM`) lacks the UI and flow for the user addition/enrollment process (`stage_signup`).

## Objective
Wire the missing routes, bind the overview dashboard to real Tauri commands, and build a cohesive, well-designed form for IAM user onboarding to bridge the gap between the Web Component shell and the Rust backend.
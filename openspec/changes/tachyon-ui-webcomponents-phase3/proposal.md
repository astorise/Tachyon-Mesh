# Proposal: Web Components Migration Phase 3 - Routing Dashboard

## 1. Context and Problem Statement
With IAM and the App Shell secured via isolated Web Components, we must reintegrate the GitOps configuration interfaces. The interactive L4/L7 routing controller needs to be reintroduced, safely connecting Vanilla JS `<input>`s to the Rust/Serde validator via Tauri, without breaking Shadow DOM isolation.

## 2. Solution: The Configuration "Vertical Slice"
We will implement `<tachyon-routing-dashboard>`, the first business component injected into the App Shell's `#router-view`.
It serves as the reference "Vertical Slice": defining the architectural standard for translating a UI form into a strict `.wit` contract payload sent to the Rust Core-Host.

## 3. Expected Benefits
- **Schema-First Validation**: The UI builds a JSON payload strictly matching the `L7Route` Rust struct.
- **Zero-Panic UI**: Any Serde validation error from Rust is intercepted by IPC and gracefully displayed in the Shadow DOM.
- **Reference Blueprint**: This sets the exact pattern for the remaining 12 configuration domains (Quotas, AI orchestration, TPUs, etc.).
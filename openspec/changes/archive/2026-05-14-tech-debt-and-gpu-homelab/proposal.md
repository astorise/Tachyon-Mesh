# Proposal: Technical Debt & Advanced GPU Deployment

## Context
The post-Codex audit identified a few remaining P2 technical debt items. While the UI now enforces a strict CSP, developers continue to use `innerHTML` in new components (`TachyonStoragePanel`, `TachyonAppShellModalRoot`), which leaves a lingering XSS risk if future interpolation bypasses our sanitizers. In the backend, the MCP server contains duplicate blocking calls and ineffective connection caching. Finally, the project touts AI/GPU capabilities, but our provided `deploy.yaml` lacks the Kubernetes primitives needed to actually schedule GPU workloads.

## Problem
1. **Latent UI Vulnerability:** Relying on `innerHTML` with template literals requires constant developer vigilance. One mistake equals an XSS exploit.
2. **MCP Inefficiency:** `read_local_hardware_status` is duplicated across handlers, and the connection cache is initialized but bypassed during the handshake.
3. **Infrastructure Disconnect:** Users running local K3s/Talos AI clusters cannot easily deploy Tachyon-Mesh because the default manifest lacks `nodeSelector: nvidia.com/gpu`, PVCs for model caching, and RBAC permissions.

## Proposed Solution
1. **DOM API Migration:** Systematically replace all remaining `innerHTML` calls in the UI with standard `document.createElement`, `textContent`, and `.replaceChildren()`.
2. **MCP Code Cleanup:** Refactor `tachyon-mcp/src/main.rs` to deduplicate hardware polling, enforce the connection cache short-circuit, and remove dead code.
3. **GPU Homelab Manifest:** Introduce a new `manifests/deploy-gpu-homelab.yaml` containing advanced K8s configuration tailored for AI workloads.

## Impact
- **Security:** Completely closes the XSS attack surface, relying on the browser's native DOM safety.
- **Maintainability:** Cleaner, DRYer Rust code in the MCP server.
- **Operator Experience:** Provides a production-ready template for deploying Tachyon on AI-accelerated K8s clusters.
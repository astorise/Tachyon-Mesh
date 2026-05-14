# Proposal: Complete XSS Mitigation (DOM API Migration)

## Context
During a previous audit remediation cycle, a technical debt change (`tech-debt-and-gpu-homelab`) was archived before its frontend tasks were fully applied. A recent code scan confirms that the Tachyon-UI codebase still relies heavily on `innerHTML` and the `escapeHtml` utility for rendering components.

## Problem
Latent UI Vulnerability: Files such as `TachyonStoragePanel.ts`, `TachyonAppShell.ts`, `TachyonRoutingDashboard.ts`, and `aiOrchestration.ts` continue to use template literals and `.innerHTML` to construct the DOM. While our strict CSP provides a safety net, relying on manual string sanitization (`escapeHtml`) is fragile. A single interpolation mistake in a complex view reopens the Cross-Site Scripting (XSS) attack surface.

## Proposed Solution
Perform a surgical strike on the frontend codebase to migrate entirely to the native, safe DOM API.
1. Eradicate all instances of `innerHTML` for dynamic content generation.
2. Replace string templates with `document.createElement()`, `element.textContent`, and `element.replaceChildren()`.
3. Delete the obsolete `escapeHtml` utility function once it is no longer referenced.

## Impact
- **Security:** Achieves structural immunity to DOM-based XSS attacks.
- **Maintainability:** Removes the cognitive load of remembering to manually escape user inputs before rendering.
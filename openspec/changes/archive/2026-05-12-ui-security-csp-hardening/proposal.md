# Proposal: Tachyon-UI Security Hardening (CSP & XSS)

## Context
A recent usability and security audit highlighted a critical P0 vulnerability in the Tachyon-UI application. The application currently runs with the Content Security Policy (CSP) entirely disabled (`"csp": null` in `tauri.conf.json`), combined with extensive use of `.innerHTML` for rendering user-controlled data (such as URLs and cluster descriptions) inside `TachyonIAM.ts` and other Web Components.

## Problem
The absence of a CSP allows the execution of unauthorized scripts. Because `.innerHTML` evaluates HTML tags, any malicious payload returned by a compromised cluster, or a crafted URL provided during the onboarding phase, can result in a Cross-Site Scripting (XSS) attack. This compromises the UI shell, the IOTA Stronghold vault, and the user's PATs.

## Proposed Solution
1. **Enforce a Strict CSP:** Define a robust CSP in `tauri.conf.json` that restricts script execution to the local context and limits network requests to authorized local/remote cluster endpoints.
2. **Eradicate Unsafe DOM Manipulation:** Ban the use of `.innerHTML` globally across the frontend codebase. Replace all instances with `textContent` or safe DOM creation APIs (`document.createElement`, `document.createTextNode`).

## Impact
- **Security:** Closes the most critical attack vector in the UI.
- **Developer Experience:** Enforces safer coding standards for future Web Components.
- **Breaking Changes:** Minor risk if some third-party visual assets or inline scripts (like specific GSAP animations) were unknowingly relying on `unsafe-inline`.
# Proposal: Web Components Migration Phase 1 - IAM Isolation

## 1. Context and Problem Statement
Tachyon Mesh relies on an ultra-lightweight UI stack (Vanilla JS + GSAP). However, the current approach to global DOM management in `main.ts` is fragile. The recent architectural rollback demonstrated that managing multiple nested views (App Shell, Dashboard) risks conflicting with the existing authentication system (IAM) and accidentally overwriting its event listeners.

## 2. Solution: "Component-First" Architecture
We propose migrating the UI to native **Web Components** (Custom Elements). 
Phase 1 involves encapsulating the currently functional IAM block into a `<tachyon-iam>` component, without altering its visual appearance or business logic.

## 3. Expected Benefits
- **Total Isolation**: Thanks to the Shadow DOM, the IAM's HTML and CSS are protected and "sandboxed" from the rest of the application.
- **Security (Auth Guard)**: The IAM component becomes the single source of truth rendered at startup, gating all subsequent views.
- **Strict Contract**: The authentication state communicates with the outside world (the future AppShell) strictly via typed `CustomEvent`s.
- **Standardization**: This creates a reusable boilerplate for Phase 2 (App Shell) and Phase 3 (L4/L7 Routing).

## 4. Styling Strategy
We will use **Constructable Stylesheets** to inject the compiled Tailwind CSS file directly into the Shadow DOM efficiently, preventing memory bloat from duplicating the stylesheet across multiple components.
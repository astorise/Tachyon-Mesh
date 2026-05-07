# Proposal: Web Components Migration Phase 2 - App Shell & Navigation

## 1. Context and Problem Statement
Phase 1 secured and isolated the authentication flow into an autonomous Web Component (`<tachyon-iam>`). We now need to reintroduce the main application interface (Sidebar, Topbar, Content Area) that was removed during the rollback, while avoiding initial state conflicts.

## 2. Solution: Reactive App Shell
We propose the creation of a `<tachyon-app-shell>` component.
This component acts as the main application layout. On initial load, it will be physically present in the DOM but **visually hidden** (via CSS). It will only transition into view upon intercepting the `iam:authenticated` event emitted by the IAM component.

## 3. Expected Benefits
- **Strict Decoupling**: The App Shell has zero knowledge of the authentication logic. It solely reacts to an event contract (DOM-level Schema-First approach).
- **Fluid Transitions**: GSAP animation orchestration between the Login disappearance and Dashboard appearance is centralized.
- **Routing Foundation**: The shell provides a main container (`#router-view`) ready to host future Vertical Slices (e.g., Phase 3: L4/L7 Routing).
# tauri-configurator Specification

## ADDED Requirements

### Requirement: The UI MUST operate as a Single Page Application without heavy DOM frameworks
The Tachyon-UI client SHALL use a lightweight Vanilla JS routing mechanism based on URL hashes (`hashchange`) to navigate between configuration domains (Routing, Security, etc.) without triggering a full browser reload.

#### Scenario: Navigating between configuration domains
- **GIVEN** the user is viewing the Dashboard at `#/dashboard`
- **WHEN** the user clicks the "Routing & Gateways" sidebar link changing the hash to `#/routing`
- **THEN** the Vanilla JS Router intercepts the event, destroys the dashboard DOM, and injects the routing DOM
- **AND** the GSAP animation engine performs a fluid cross-fade transition without relying on a Virtual DOM diffing algorithm.

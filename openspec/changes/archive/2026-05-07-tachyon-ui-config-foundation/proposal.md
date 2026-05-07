# UI Configuration Foundation

## Why
Tachyon UI needs a standardized way to build configuration panels while preserving the Zero-Panic policy and keeping IAM isolated from configuration-domain UI code. Adding new views directly to the shell or global DOM risks overwriting event listeners, leaking styles, and duplicating error-handling behavior across domains.

## What Changes
- Add a shared `TachyonConfigDashboard` base class for configuration Web Components.
- Add shared Tailwind constructable stylesheet plumbing for dashboard components.
- Add standardized success/error feedback rendering for handled Tauri and Rust responses.
- Add a component registry that maps App Shell sidebar routes to dashboard custom element tags.

## Impact
Configuration dashboards will use a common Shadow DOM, styling, animation, and feedback foundation. This provides a repeatable pattern for future configuration domains without coupling them to IAM or global document selectors.

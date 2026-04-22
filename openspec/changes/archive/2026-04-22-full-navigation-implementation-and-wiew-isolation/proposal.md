# Proposal: full-navigation-implementation-and-wiew-isolation

## Why
The desktop shell already contains most of the management-plane widgets required by Tachyon Studio, but the navigation model still mixes legacy view names with newer surfaces. The sidebar does not clearly reflect dedicated Registry, Broker, Identity, and personal Account planes, which makes the UI architecture harder to reason about and leaves the OpenSpec change blocked on invalid artifacts.

## What Changes
- Normalize the navigation change into valid OpenSpec delta artifacts.
- Align the frontend view IDs and router state with explicit isolated surfaces: dashboard, topology, registry, identity, account, and broker.
- Keep `get_mesh_graph` as the stable topology bridge in the Tauri wrapper and ensure navigation transitions remain SPA-based.

## Impact
- `tachyon-ui/index.html` and `tachyon-ui/src/main.ts` become the canonical sources for view isolation.
- The existing topology, asset upload, identity, and model broker widgets are preserved but reassigned to stable view IDs.
- The change composes with the IAM change by adding a dedicated personal account surface.

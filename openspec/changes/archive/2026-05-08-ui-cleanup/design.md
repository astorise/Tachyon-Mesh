## Context

Tachyon UI now uses Shadow DOM Web Components for authentication, shell layout, route rendering, notifications, and domain panels. The legacy Light DOM application still existed in parallel and could race with the Web Component shell.

## Decisions

- Keep `index.html` as a minimal host document with only `#app-root`, `#auth-layer`, `<tachyon-iam>`, and `<tachyon-app-shell>`.
- Move `<tachyon-toast-manager>` inside `<tachyon-app-shell>` so the document body stays minimal while global notifications remain connected.
- Delete `router.ts` and let `TachyonAppShell` own hash routing.
- Normalize route hashes by stripping `#` and a leading `/`, then resolve only `dashboard` or entries listed by `ComponentRegistry`.
- Replace the legacy `main.ts` with a small bootstrapper that imports CSS/components, initializes the Zustand connection store, and listens for Tauri mesh connectivity events.

## Trade-offs

- The dashboard remains a static shell panel, while all configuration/domain routes are resolved through `ComponentRegistry`.
- Legacy Light DOM views are removed rather than gradually migrated; missing workflows must exist as Web Components to remain reachable.

# Title: Legacy UI Exorcism and Router Unification

## Problem Statement
The Tachyon-UI currently suffers from a "split-brain" architecture. While the new Shadow DOM Web Components (`<tachyon-app-shell>`, `<tachyon-iam>`) have been introduced, the legacy Light DOM UI (sidebar, modals, and 2200+ lines of imperative DOM manipulation in `main.ts`) remains active. Furthermore, two competing routing systems exist: the legacy `router.ts` (handling 4 routes) and the new `ComponentRegistry.ts` (handling 14 routes). This causes race conditions, double-rendering ("zombie UI"), and a massive bundle bloat.

## Objective
1. Purge all legacy Light DOM elements from `index.html`, leaving only the necessary Web Component mounts.
2. Delete `router.ts` and strictly unify all navigation under `ComponentRegistry.ts` and `TachyonAppShell.ts`.
3. Exorcise `main.ts` of all legacy view rendering logic, reducing it to a lean bootstrapper for Tauri, global state (Zustand), and Web Component initialization.
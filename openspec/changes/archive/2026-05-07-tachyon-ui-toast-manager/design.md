## Context

Domain dashboard panels currently show feedback only inside their local shadow DOM. When the operator navigates away or a panel is partially off-screen, success and error feedback can be missed. The shell needs a global, non-blocking notification surface.

## Goals / Non-Goals

**Goals:**
- Add a `<tachyon-toast-manager>` web component mounted outside routed shell content.
- Listen for global `app:notify` events and render dismissible toasts.
- Reuse shared Tachyon styling and GSAP entrance/exit animations.
- Dispatch `app:notify` from `TachyonConfigDashboard.showFeedback` alongside local feedback.

**Non-Goals:**
- Replace local `feedback-zone` content.
- Add server-side notification persistence.
- Implement a notification history drawer.

## Decisions

- Mount the toast manager next to `<tachyon-app-shell>` in `index.html` so it remains available regardless of current route.
- Use `window.dispatchEvent` / `window.addEventListener` for shell-wide notifications without coupling dashboards to a singleton instance.
- Keep toast content escaped and bounded to avoid rendering arbitrary HTML from Rust or validation errors.

## Risks / Trade-offs

- Every dashboard feedback event now creates both local and global feedback. Mitigation: toasts auto-dismiss and local feedback remains useful for context.
- Global events can be emitted by any frontend code. Mitigation: the toast manager escapes message text and ignores malformed event details.

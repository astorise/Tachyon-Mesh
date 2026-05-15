const FOCUSABLE =
  'a[href], button:not([disabled]), textarea, input:not([type="hidden"]):not([disabled]), select, [tabindex]:not([tabindex="-1"])';

/**
 * Traps keyboard focus inside `element`.
 * - Tab / Shift+Tab cycle through focusable children.
 * - Escape calls `onClose()` if provided.
 * - Immediately focuses the first focusable child.
 * - On cleanup, restores focus to the element that was active before trapping.
 * Returns a cleanup function that removes the keydown listener and restores focus.
 *
 * ⚠️ Do NOT use with native `<dialog>` elements — the browser's built-in focus
 * trap conflicts with this utility and produces duplicate or unpredictable behaviour.
 * Use `<dialog>.showModal()` directly for native dialogs.
 */
export function trapFocus(element: HTMLElement, onClose?: () => void): () => void {
  const previousFocus = document.activeElement as HTMLElement | null;
  const getFocusable = (): HTMLElement[] =>
    Array.from(element.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
      (el) => !el.closest("[hidden]") && el.offsetParent !== null,
    );

  const handleKeyDown = (event: KeyboardEvent): void => {
    if (event.key === "Escape" || event.key === "Esc") {
      if (onClose) {
        onClose();
        event.preventDefault();
      }
      return;
    }

    if (event.key !== "Tab") return;

    const focusable = getFocusable();
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];

    if (event.shiftKey) {
      if (document.activeElement === first) {
        last.focus();
        event.preventDefault();
      }
    } else {
      if (document.activeElement === last) {
        first.focus();
        event.preventDefault();
      }
    }
  };

  element.addEventListener("keydown", handleKeyDown);

  const firstFocusable = getFocusable()[0];
  if (firstFocusable) {
    firstFocusable.focus();
  }

  return () => {
    element.removeEventListener("keydown", handleKeyDown);
    previousFocus?.focus();
  };
}

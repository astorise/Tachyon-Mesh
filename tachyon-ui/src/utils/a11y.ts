const FOCUSABLE =
  'a[href], button:not([disabled]), textarea, input:not([type="hidden"]):not([disabled]), select, [tabindex]:not([tabindex="-1"])';

/**
 * Traps keyboard focus inside `element`.
 * - Tab / Shift+Tab cycle through focusable children.
 * - Escape calls `onClose()` if provided.
 * - Immediately focuses the first focusable child.
 * Returns a cleanup function that removes the keydown listener.
 */
export function trapFocus(element: HTMLElement, onClose?: () => void): () => void {
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

  return () => element.removeEventListener("keydown", handleKeyDown);
}

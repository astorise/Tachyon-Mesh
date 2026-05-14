import "./TachyonBundleConflictModal";
import "./TachyonGuidedTour";
import "./TachyonToastManager";
import { trapFocus } from "../../utils/a11y";
import { tachyonSharedStylesheet } from "../../styles/shared-sheets";

type BundleConflict = {
  name: string;
  bundledVersion: string;
  clusterVersion: string;
};

type BundleConflictModalElement = HTMLElement & {
  open: (conflicts: BundleConflict[]) => void;
};

type GuidedTourElement = HTMLElement & {
  start: () => void;
  startIfFirstVisit: () => void;
};

/**
 * Modal and overlay orchestrator extracted from TachyonAppShell.
 *
 * Manages the z-index stack for all full-screen overlays:
 * - `<tachyon-toast-manager>` — ephemeral notification toasts
 * - `<tachyon-guided-tour>` — step-by-step onboarding tour
 * - `<tachyon-bundle-conflict-modal>` — manifest bundle conflict resolution dialog
 *
 * Listens to the `topology:conflict` window event and opens the conflict modal
 * automatically. Parent components can also call `openConflictModal`, `startTour`,
 * and `startTourIfFirstVisit` directly.
 */
export class TachyonAppShellModalRoot extends HTMLElement {
  private readonly root: ShadowRoot;
  private readonly onTopologyConflict = (event: Event) => {
    const conflicts = (event as CustomEvent<{ conflicts: BundleConflict[] }>).detail.conflicts;
    this.openConflictModal(conflicts);
  };

  constructor() {
    super();
    this.root = this.attachShadow({ mode: "open" });
    this.root.adoptedStyleSheets = [tachyonSharedStylesheet];
  }

  connectedCallback(): void {
    this.root.innerHTML = `
      <tachyon-guided-tour></tachyon-guided-tour>
      <tachyon-toast-manager></tachyon-toast-manager>
      <tachyon-bundle-conflict-modal></tachyon-bundle-conflict-modal>
    `;
    window.addEventListener("topology:conflict", this.onTopologyConflict);
  }

  disconnectedCallback(): void {
    window.removeEventListener("topology:conflict", this.onTopologyConflict);
  }

  /** Open the bundle conflict resolution modal and trap keyboard focus inside it. */
  openConflictModal(conflicts: BundleConflict[]): void {
    const modal = this.conflictModal();
    if (!modal) return;
    modal.setAttribute("role", "dialog");
    modal.setAttribute("aria-modal", "true");
    modal.open(conflicts);
    // Escape closes the conflict modal.
    trapFocus(modal, () => {
      modal.removeAttribute("role");
      modal.removeAttribute("aria-modal");
    });
  }

  /** Start the guided tour from step 0. */
  startTour(): void {
    this.guidedTour()?.start();
  }

  /** Start the guided tour only if this is the operator's first visit. */
  startTourIfFirstVisit(): void {
    this.guidedTour()?.startIfFirstVisit();
  }

  private conflictModal(): BundleConflictModalElement | null {
    return this.root.querySelector<BundleConflictModalElement>("tachyon-bundle-conflict-modal");
  }

  private guidedTour(): GuidedTourElement | null {
    return this.root.querySelector<GuidedTourElement>("tachyon-guided-tour");
  }
}

customElements.define("tachyon-app-shell-modal-root", TachyonAppShellModalRoot);

import gsap from "gsap";

import { tachyonSharedStylesheet } from "../../styles/shared-sheets";

export type FeedbackKind = "success" | "error";

export abstract class TachyonConfigDashboard extends HTMLElement {
  protected readonly root: ShadowRoot;

  constructor() {
    super();
    this.root = this.attachShadow({ mode: "open" });
    this.applyStyles();
  }

  protected renderTemplate(html: string): void {
    this.root.innerHTML = html;
  }

  protected applyStyles(): void {
    this.root.adoptedStyleSheets = [tachyonSharedStylesheet];
  }

  protected showFeedback(type: FeedbackKind, message: string): void {
    window.dispatchEvent(new CustomEvent("app:notify", { detail: { type, message } }));

    const zone = this.root.getElementById("feedback-zone");
    if (!zone) {
      return;
    }

    const tone =
      type === "success"
        ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-300"
        : "border-red-500/30 bg-red-500/10 text-red-300";
    const feedback = document.createElement("div");
    feedback.className = `rounded-lg border px-4 py-3 ${tone}`;
    feedback.textContent = message;
    zone.replaceChildren(feedback);
    void gsap.fromTo(zone, { opacity: 0, y: 10 }, { opacity: 1, y: 0, duration: 0.24, ease: "power2.out" });
    if (type === "success") {
      void gsap.fromTo(
        zone,
        { scale: 0.985, boxShadow: "0 0 0 rgba(16,185,129,0)" },
        {
          scale: 1,
          boxShadow: "0 0 24px rgba(16,185,129,0.22)",
          duration: 0.28,
          yoyo: true,
          repeat: 1,
          ease: "power2.out",
        },
      );
    }
  }

  /**
   * Wraps an async data-fetch task with skeleton loading UI.
   *
   * While `task` is pending the shadow root (or the element matched by
   * `containerSelector`) shows shimmer skeleton blocks.  On success the result
   * is returned; on failure `handlePanelError` is called, which fires an
   * actionable error toast with an inline Retry button.
   */
  protected async withLoadingState<T>(
    task: () => Promise<T>,
    containerSelector?: string,
  ): Promise<T | undefined> {
    const container: Element | null = containerSelector
      ? this.root.querySelector(containerSelector)
      : this.root.firstElementChild;

    if (container) {
      container.innerHTML = `
        <div class="p-6 w-full space-y-3" aria-busy="true" aria-label="Loading…">
          <div class="skeleton-text w-1/3"></div>
          <div class="skeleton-block"></div>
          <div class="skeleton-text w-1/2"></div>
          <div class="skeleton-text w-2/3 mt-4"></div>
        </div>
      `;
    }

    try {
      return await task();
    } catch (error) {
      this.handlePanelError(error, task);
      return undefined;
    }
  }

  /**
   * Clears the skeleton and dispatches an actionable error toast.
   * If `retryTask` is provided the toast includes a "Retry" button that
   * re-invokes `withLoadingState` from scratch.
   */
  protected handlePanelError(error: unknown, retryTask?: () => Promise<unknown>): void {
    const message = error instanceof Error ? error.message : String(error);
    const action = retryTask
      ? {
          label: "Retry",
          onClick: () => {
            void this.withLoadingState(retryTask);
          },
        }
      : undefined;
    window.dispatchEvent(
      new CustomEvent("toast", {
        detail: { type: "error", message, action },
      }),
    );
  }

  protected animateEntrance(): void {
    const panels = this.root.querySelectorAll<HTMLElement>("[data-stagger-panel]");
    if (panels.length === 0) {
      return;
    }
    void gsap.fromTo(
      panels,
      { y: 16, opacity: 0 },
      { y: 0, opacity: 1, duration: 0.32, stagger: 0.06, ease: "power2.out" },
    );
  }
}

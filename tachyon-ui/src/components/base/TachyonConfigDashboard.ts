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
    const zone = this.root.getElementById("feedback-zone");
    if (!zone) {
      return;
    }

    const tone =
      type === "success"
        ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-300"
        : "border-red-500/30 bg-red-500/10 text-red-300";
    zone.innerHTML = `<div class="rounded-lg border px-4 py-3 ${tone}">${this.escapeHtml(message)}</div>`;
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

  private escapeHtml(value: string): string {
    return value
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#039;");
  }
}

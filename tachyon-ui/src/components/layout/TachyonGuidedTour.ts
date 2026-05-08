import gsap from "gsap";

import stylesheetText from "../../style.css?inline";
import { t } from "../../utils/i18n";

type TourStep = {
  target: string;
  titleKey: string;
  contentKey: string;
};

const tourStylesheet = new CSSStyleSheet();
tourStylesheet.replaceSync(stylesheetText);
const tourCompletedKey = "tachyon_tour_completed";

const steps: TourStep[] = [
  { target: "#shell-user", titleKey: "tour.auth.title", contentKey: "tour.auth.desc" },
  { target: "#shell-header", titleKey: "tour.header.title", contentKey: "tour.header.desc" },
  { target: "#btn-seal-apply", titleKey: "tour.seal.title", contentKey: "tour.seal.desc" },
  { target: '[data-route-panel="dynamic"] [data-metric="nodes"]', titleKey: "tour.overview.title", contentKey: "tour.overview.desc" },
  { target: '[data-route="observability"]', titleKey: "tour.observability.title", contentKey: "tour.observability.desc" },
];

export class TachyonGuidedTour extends HTMLElement {
  private readonly root: ShadowRoot;
  private currentIndex = 0;
  private running = false;

  constructor() {
    super();
    this.root = this.attachShadow({ mode: "open" });
    this.root.adoptedStyleSheets = [tourStylesheet];
  }

  connectedCallback(): void {
    this.render();
    this.bindEvents();
  }

  start(): void {
    this.currentIndex = 0;
    this.running = true;
    this.render();
    this.bindEvents();
    this.updateStep();
  }

  startIfFirstVisit(): void {
    if (localStorage.getItem(tourCompletedKey) === "true") {
      return;
    }
    window.setTimeout(() => this.start(), 450);
  }

  private bindEvents(): void {
    this.root.getElementById("tour-skip")?.addEventListener("click", () => this.finish());
    this.root.getElementById("tour-prev")?.addEventListener("click", () => this.previous());
    this.root.getElementById("tour-next")?.addEventListener("click", () => this.next());
  }

  private render(): void {
    this.root.innerHTML = `
      <section id="tour-layer" class="${this.running ? "fixed" : "hidden"} inset-0 z-[150] bg-slate-950/80 text-slate-200 backdrop-blur-sm">
        <div id="tour-highlight" class="pointer-events-none fixed rounded-lg border-2 border-cyan-400 shadow-[0_0_32px_rgba(34,211,238,0.45)]"></div>
        <dialog id="tour-dialog" open class="fixed m-0 w-[min(24rem,calc(100vw-2rem))] rounded-lg border border-slate-700 bg-slate-900 p-5 text-slate-200 shadow-2xl">
          <h3 id="tour-title" class="text-lg font-semibold text-cyan-300"></h3>
          <p id="tour-content" class="mt-3 text-sm leading-6 text-slate-400"></p>
          <div class="mt-5 flex items-center justify-between gap-3">
            <button id="tour-skip" type="button" class="rounded border border-slate-700 px-3 py-2 text-xs font-medium text-slate-300 hover:bg-slate-800">${t("tour.skip")}</button>
            <div class="flex gap-2">
              <button id="tour-prev" type="button" class="rounded border border-slate-700 px-3 py-2 text-xs font-medium text-slate-300 hover:bg-slate-800">${t("tour.previous")}</button>
              <button id="tour-next" type="button" class="rounded border border-cyan-500/60 bg-cyan-500/15 px-3 py-2 text-xs font-semibold text-cyan-200 hover:bg-cyan-500/25">${t("tour.next")}</button>
            </div>
          </div>
        </dialog>
      </section>
    `;
  }

  private next(): void {
    if (this.currentIndex >= steps.length - 1) {
      this.finish();
      return;
    }
    this.currentIndex += 1;
    this.updateStep();
  }

  private previous(): void {
    this.currentIndex = Math.max(0, this.currentIndex - 1);
    this.updateStep();
  }

  private finish(): void {
    localStorage.setItem(tourCompletedKey, "true");
    this.running = false;
    this.render();
  }

  private updateStep(): void {
    const step = steps[this.currentIndex];
    const title = this.root.getElementById("tour-title");
    const content = this.root.getElementById("tour-content");
    const next = this.root.getElementById("tour-next");
    const prev = this.root.getElementById("tour-prev") as HTMLButtonElement | null;
    if (title) {
      title.textContent = t(step.titleKey);
    }
    if (content) {
      content.textContent = t(step.contentKey);
    }
    if (next) {
      next.textContent = this.currentIndex === steps.length - 1 ? t("tour.finish") : t("tour.next");
    }
    if (prev) {
      prev.disabled = this.currentIndex === 0;
      prev.classList.toggle("opacity-40", prev.disabled);
    }
    this.positionAroundTarget(step.target);
  }

  private positionAroundTarget(selector: string): void {
    const rootNode = this.getRootNode() as Document | ShadowRoot;
    const target = rootNode.querySelector<HTMLElement>(selector) ?? rootNode.querySelector<HTMLElement>("#shell-header") ?? rootNode.querySelector<HTMLElement>("#shell-sidebar");
    const highlight = this.root.getElementById("tour-highlight");
    const dialog = this.root.getElementById("tour-dialog");
    if (!target || !highlight || !dialog) {
      return;
    }

    const rect = target.getBoundingClientRect();
    const padding = 8;
    const left = Math.max(12, rect.left - padding);
    const top = Math.max(12, rect.top - padding);
    const width = Math.min(window.innerWidth - left - 12, rect.width + padding * 2);
    const height = Math.min(window.innerHeight - top - 12, rect.height + padding * 2);
    gsap.set(highlight, { left, top, width, height });
    void gsap.fromTo(highlight, { opacity: 0.4, scale: 0.98 }, { opacity: 1, scale: 1, duration: 0.24, ease: "power2.out" });

    const dialogLeft = Math.min(Math.max(16, left + width + 16), window.innerWidth - 400);
    const dialogTop = Math.min(Math.max(16, top), window.innerHeight - 260);
    gsap.set(dialog, { left: dialogLeft, top: dialogTop });
    void gsap.fromTo(dialog, { opacity: 0, y: 12 }, { opacity: 1, y: 0, duration: 0.22, ease: "power2.out" });
  }
}

customElements.define("tachyon-guided-tour", TachyonGuidedTour);

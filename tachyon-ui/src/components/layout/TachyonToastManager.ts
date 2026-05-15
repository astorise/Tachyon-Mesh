import gsap from "gsap";

import { tachyonSharedStylesheet } from "../../styles/shared-sheets";

type ToastKind = "success" | "error" | "warning" | "info";

export type ToastAction = {
  label: string;
  onClick: () => void;
};

export type ToastDetail = {
  type?: ToastKind;
  message?: string;
  /** Optional inline action button rendered next to the message. */
  action?: ToastAction;
};

const toastStylesheet = new CSSStyleSheet();
toastStylesheet.replaceSync(`
  :host {
    pointer-events: none;
  }
`);

export class TachyonToastManager extends HTMLElement {
  private readonly root: ShadowRoot;
  private readonly onNotify = (event: Event) => {
    const detail = (event as CustomEvent<ToastDetail>).detail;
    this.createToast(detail);
  };

  constructor() {
    super();
    this.root = this.attachShadow({ mode: "open" });
    this.root.adoptedStyleSheets = [tachyonSharedStylesheet, toastStylesheet];
  }

  connectedCallback(): void {
    this.render();
    window.addEventListener("app:notify", this.onNotify);
    window.addEventListener("toast", this.onNotify);
  }

  disconnectedCallback(): void {
    window.removeEventListener("app:notify", this.onNotify);
    window.removeEventListener("toast", this.onNotify);
  }

  private render(): void {
    this.root.innerHTML = `
      <div id="toast-container" role="status" aria-live="polite" aria-atomic="false" class="fixed bottom-6 right-6 z-[120] flex max-w-[min(28rem,calc(100vw-3rem))] flex-col gap-3 pointer-events-none"></div>
    `;
  }

  private createToast(detail: ToastDetail | undefined): void {
    const message = detail?.message?.trim();
    if (!message) {
      return;
    }

    const container = this.root.getElementById("toast-container");
    if (!container) {
      return;
    }

    const type: ToastKind = detail?.type === "error" ? "error"
      : detail?.type === "warning" ? "warning"
      : detail?.type === "info" ? "info"
      : "success";

    const tone =
      type === "error"   ? "border-red-500/60 bg-red-950/90 text-red-100"
      : type === "warning" ? "border-amber-500/60 bg-amber-950/90 text-amber-100"
      : type === "info"    ? "border-blue-500/60 bg-blue-950/90 text-blue-100"
      :                      "border-cyan-500/60 bg-slate-900/95 text-cyan-100";

    const marker =
      type === "error" ? "!" : type === "warning" ? "!" : type === "info" ? "i" : "OK";

    const toast = document.createElement("div");
    toast.className = `pointer-events-auto flex min-w-72 items-start gap-3 rounded-lg border px-4 py-3 shadow-xl shadow-black/50 backdrop-blur-sm ${tone}`;

    const markerNode = document.createElement("span");
    markerNode.className =
      "mt-0.5 inline-flex h-6 min-w-6 items-center justify-center rounded border border-current px-1 font-mono text-[10px] font-bold";
    markerNode.textContent = marker;

    const body = document.createElement("p");
    body.className = "min-w-0 flex-1 text-sm font-medium leading-5";
    body.textContent = message;

    toast.append(markerNode, body);

    // Render the optional action button.
    if (detail?.action) {
      const action = detail.action;
      const actionBtn = document.createElement("button");
      actionBtn.type = "button";
      actionBtn.className =
        "shrink-0 self-center rounded border border-current/40 bg-white/10 px-3 py-1 text-xs font-semibold hover:bg-white/20 transition-colors";
      actionBtn.textContent = action.label;
      actionBtn.addEventListener("click", () => {
        action.onClick();
        this.dismissToast(toast);
      });
      toast.append(actionBtn);
    }

    const dismiss = document.createElement("button");
    dismiss.type = "button";
    dismiss.className =
      "shrink-0 self-start rounded px-1 font-mono text-xs opacity-70 transition-opacity hover:opacity-100";
    dismiss.setAttribute("aria-label", "Dismiss notification");
    dismiss.textContent = "×";
    toast.append(dismiss);

    dismiss.addEventListener("click", () => {
      this.dismissToast(toast);
    });
    container.appendChild(toast);

    void gsap.fromTo(
      toast,
      { opacity: 0, x: 48, scale: 0.96 },
      { opacity: 1, x: 0, scale: 1, duration: 0.32, ease: "back.out(1.5)" },
    );

    // Action toasts stay longer so the user has time to click.
    const ttl = detail?.action ? 8000 : 4000;
    window.setTimeout(() => this.dismissToast(toast), ttl);
  }

  private dismissToast(toast: HTMLElement): void {
    if (!toast.isConnected) {
      return;
    }
    void gsap.to(toast, {
      opacity: 0,
      x: 48,
      duration: 0.2,
      ease: "power2.in",
      onComplete: () => toast.remove(),
    });
  }
}

customElements.define("tachyon-toast-manager", TachyonToastManager);

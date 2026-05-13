import { tachyonSharedStylesheet } from "../../styles/shared-sheets";
import { listComponentRoutes } from "../../registry/ComponentRegistry";
import { t } from "../../utils/i18n";

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#039;");
}

/**
 * Sidebar navigation component extracted from TachyonAppShell.
 *
 * Observes the `active-route` attribute. On each nav-button click,
 * dispatches `shell:navigate` with `{ route }` so the parent shell can
 * update the main-content area without knowing the sidebar implementation.
 */
export class TachyonAppShellNav extends HTMLElement {
  private readonly root: ShadowRoot;
  private readonly onLanguageChanged = () => this.render();

  static get observedAttributes() {
    return ["active-route"];
  }

  constructor() {
    super();
    this.root = this.attachShadow({ mode: "open" });
    this.root.adoptedStyleSheets = [tachyonSharedStylesheet];
  }

  connectedCallback(): void {
    window.addEventListener("i18n:language-changed", this.onLanguageChanged);
    this.render();
  }

  disconnectedCallback(): void {
    window.removeEventListener("i18n:language-changed", this.onLanguageChanged);
  }

  attributeChangedCallback(): void {
    this.render();
    this.updateActiveState();
  }

  private get activeRoute(): string {
    return this.getAttribute("active-route") ?? "dashboard";
  }

  private render(): void {
    const active = this.activeRoute;
    const configLinks = listComponentRoutes()
      .map(
        (entry) =>
          `<button data-route="${escapeHtml(entry.route)}" class="nav-link w-full text-left block px-4 py-2 rounded-md text-slate-300 hover:bg-slate-800/50 transition-colors">${escapeHtml(t(`nav.${entry.route}`) || entry.label)}</button>`,
      )
      .join("");

    this.root.innerHTML = `
      <aside class="w-64 bg-slate-900 border-r border-slate-800 flex flex-col h-screen" aria-label="${t("shell.sidebar-label")}">
        <div class="h-16 flex items-center px-6 border-b border-slate-800">
          <div class="w-3 h-3 bg-cyan-400 rounded-full mr-3 shadow-[0_0_10px_rgba(34,211,238,0.8)]" aria-hidden="true"></div>
          <h1 class="text-xl font-bold text-white tracking-wider">TACHYON<span class="text-cyan-400">MESH</span></h1>
        </div>
        <nav class="flex-1 p-4 space-y-2" aria-label="${t("shell.nav-label")}">
          <button data-route="dashboard" class="nav-link w-full text-left block px-4 py-2 rounded-md ${active === "dashboard" ? "bg-slate-800 text-cyan-400 font-medium" : "text-slate-300"} transition-colors">${t("nav.dashboard")}</button>
          ${configLinks}
        </nav>
        <div class="p-4 border-t border-slate-800 text-xs text-slate-500" aria-hidden="true">${t("shell.version")}</div>
      </aside>
    `;

    this.root.querySelectorAll<HTMLButtonElement>("[data-route]").forEach((button) => {
      button.addEventListener("click", () => {
        const route = button.dataset.route ?? "dashboard";
        this.dispatchEvent(
          new CustomEvent("shell:navigate", {
            bubbles: true,
            composed: true,
            detail: { route },
          }),
        );
        if (window.location.hash.slice(1) !== route) {
          window.location.hash = route;
        }
      });
    });

    this.updateActiveState();
  }

  private updateActiveState(): void {
    const active = this.activeRoute;
    this.root.querySelectorAll<HTMLButtonElement>("[data-route]").forEach((button) => {
      const isActive = button.dataset.route === active;
      button.classList.toggle("bg-slate-800", isActive);
      button.classList.toggle("text-cyan-400", isActive);
      button.classList.toggle("font-medium", isActive);
      button.classList.toggle("text-slate-300", !isActive);
    });
  }
}

customElements.define("tachyon-app-shell-nav", TachyonAppShellNav);

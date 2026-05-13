import gsap from "gsap";

import "../domains/TachyonAIPanel";
import "../domains/TachyonTopologyPanel";
import "../domains/TachyonUsersPanel";
import "../domains/TachyonHardwarePanel";
import "../domains/TachyonIdentityPanel";
import "../domains/TachyonObservabilityPanel";
import "../domains/TachyonOverviewPanel";
import "../domains/TachyonRbacPanel";
import "../domains/TachyonFleetPanel";
import "../domains/TachyonStoragePanel";
import "../domains/TachyonSupplyChainPanel";
import "../domains/TachyonWorkloadsPanel";
import "../iam/TachyonIAM";
import "./TachyonBundleConflictModal";
import "./TachyonGuidedTour";
import "./TachyonToastManager";
import "../routing/TachyonRoutingDashboard";
import "../traffic/TachyonResiliencePanel";
import "../traffic/TachyonRoutingPanel";
import stylesheetText from "../../style.css?inline";
import { listComponentRoutes, resolveComponentTag } from "../../registry/ComponentRegistry";
import { getLanguage, setLanguage, t } from "../../utils/i18n";
import { resilientInvoke as invoke } from "../../utils/network";

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#039;");
}

type AuthenticatedDetail = {
  user: string;
  role: string;
  token: string;
};

type GuidedTourElement = HTMLElement & {
  start: () => void;
  startIfFirstVisit: () => void;
};

type SealApplyOutcome = {
  success: boolean;
  message: string;
  configVersion: number;
};

type BundleConflict = {
  name: string;
  bundledVersion: string;
  clusterVersion: string;
};

type BundleApplyOutcome = {
  success: boolean;
  message: string;
  configVersion: number;
  conflicts: BundleConflict[];
  requiresResolution: boolean;
};

type BundleConflictModalElement = HTMLElement & {
  open: (conflicts: BundleConflict[]) => void;
};

const appShellStylesheet = new CSSStyleSheet();
appShellStylesheet.replaceSync(stylesheetText);

export class TachyonAppShell extends HTMLElement {
  private readonly root: ShadowRoot;
  private activeRoute = "overview";
  private activeUser = t("shell.unknown");
  private activeRole = "operator";
  private shellStarted = false;
  private requiresSeal = false;
  private applyingSeal = false;
  private readonly onAuthenticated = (event: Event) => {
    const detail = (event as CustomEvent<AuthenticatedDetail>).detail;
    void this.startTransition(detail);
  };
  private readonly onLanguageChanged = () => {
    this.render();
    if (this.shellStarted) {
      this.restoreStartedState();
    }
  };
  private readonly onConfigStaged = () => {
    this.requiresSeal = true;
    this.render();
    if (this.shellStarted) {
      this.restoreStartedState();
    }
  };
  private readonly onHashChange = () => {
    const route = this.resolveHashRoute();
    this.updateNavigation(route);
    this.showRoute(route);
  };
  private readonly onTopologyConflict = (event: Event) => {
    const conflicts = (event as CustomEvent<{ conflicts: BundleConflict[] }>).detail.conflicts;
    const modal = this.root.querySelector<BundleConflictModalElement>("tachyon-bundle-conflict-modal");
    modal?.open(conflicts);
  };

  constructor() {
    super();
    this.root = this.attachShadow({ mode: "open" });
    this.root.adoptedStyleSheets = [appShellStylesheet];
  }

  connectedCallback(): void {
    this.render();
    document.addEventListener("iam:authenticated", this.onAuthenticated);
    window.addEventListener("i18n:language-changed", this.onLanguageChanged);
    window.addEventListener("config:staged", this.onConfigStaged);
    window.addEventListener("hashchange", this.onHashChange);
    window.addEventListener("topology:conflict", this.onTopologyConflict);
  }

  disconnectedCallback(): void {
    document.removeEventListener("iam:authenticated", this.onAuthenticated);
    window.removeEventListener("i18n:language-changed", this.onLanguageChanged);
    window.removeEventListener("config:staged", this.onConfigStaged);
    window.removeEventListener("hashchange", this.onHashChange);
    window.removeEventListener("topology:conflict", this.onTopologyConflict);
  }

  async startTransition(userData: AuthenticatedDetail): Promise<void> {
    this.activeUser = userData.user;
    this.activeRole = userData.role;
    this.render();
    const authLayer = document.getElementById("auth-layer");
    const shell = this.root.getElementById("shell");
    const sidebar = this.root.getElementById("shell-sidebar");
    const header = this.root.getElementById("shell-header");
    const content = this.root.getElementById("main-content");
    const user = this.root.getElementById("shell-user");
    if (!shell || !sidebar || !header || !content || !user) {
      return;
    }

    this.activeRoute = this.resolveHashRoute();
    this.shellStarted = true;
    user.textContent = this.activeUser;
    authLayer?.classList.add("hidden");
    this.classList.remove("hidden");
    shell.classList.remove("hidden");
    shell.classList.add("flex");
    this.updateNavigation(this.activeRoute);
    this.showRoute(this.activeRoute);

    const timeline = gsap.timeline();
    timeline
      .fromTo(sidebar, { x: -50, opacity: 0 }, { x: 0, opacity: 1, duration: 0.28, ease: "power2.out" })
      .fromTo(header, { y: -20, opacity: 0 }, { y: 0, opacity: 1, duration: 0.22 }, "-=0.12")
      .fromTo(content, { opacity: 0 }, { opacity: 1, duration: 0.22 }, "-=0.08");
    await timeline.then();
    this.guidedTour()?.startIfFirstVisit();
  }

  private render(): void {
    const configLinks = listComponentRoutes()
      .map(
        (entry) =>
          `<button data-route="${escapeHtml(entry.route)}" class="nav-link w-full text-left block px-4 py-2 rounded-md text-slate-300 hover:bg-slate-800/50 transition-colors">${escapeHtml(t(`nav.${entry.route}`) || entry.label)}</button>`,
      )
      .join("");
    this.root.innerHTML = `
      <section id="shell" class="hidden fixed inset-0 z-30 h-screen w-screen bg-slate-950 text-slate-300">
        <a href="#main-content" class="skip-nav">${t("shell.skip-nav")}</a>
        <aside id="shell-sidebar" class="w-64 bg-slate-900 border-r border-slate-800 flex flex-col opacity-0" aria-label="${t("shell.sidebar-label")}">
          <div class="h-16 flex items-center px-6 border-b border-slate-800">
            <div class="w-3 h-3 bg-cyan-400 rounded-full mr-3 shadow-[0_0_10px_rgba(34,211,238,0.8)]" aria-hidden="true"></div>
            <h1 class="text-xl font-bold text-white tracking-wider">TACHYON<span class="text-cyan-400">MESH</span></h1>
          </div>
          <nav class="flex-1 p-4 space-y-2" aria-label="${t("shell.nav-label")}">
            <button data-route="dashboard" class="nav-link w-full text-left block px-4 py-2 rounded-md bg-slate-800 text-cyan-400 font-medium transition-colors">${t("nav.dashboard")}</button>
            ${configLinks}
          </nav>
          <div class="p-4 border-t border-slate-800 text-xs text-slate-500" aria-hidden="true">${t("shell.version")}</div>
        </aside>
        <div class="flex min-w-0 flex-1 flex-col">
          <header id="shell-header" class="h-16 border-b border-slate-800 flex items-center justify-between px-8 bg-slate-900/50 backdrop-blur-md opacity-0" aria-label="${t("shell.header-label")}">
            <div>
              <h2 class="text-lg font-semibold text-white">${t("shell.title")}</h2>
              <p class="text-xs text-slate-500">${t("shell.subtitle")}</p>
            </div>
            <div class="flex items-center gap-3">
              <button id="btn-seal-apply" type="button" aria-live="polite" class="${this.requiresSeal ? "inline-flex" : "hidden"} items-center rounded border border-amber-400/60 bg-amber-400/15 px-3 py-2 text-xs font-semibold uppercase tracking-wider text-amber-200 shadow-[0_0_18px_rgba(251,191,36,0.24)] transition-colors hover:bg-amber-400/25 disabled:cursor-wait disabled:opacity-70">
                ${this.applyingSeal ? "Applying..." : "Pending Changes: Seal & Apply"}
              </button>
              <label class="flex items-center gap-2 rounded border border-slate-700 bg-slate-950 px-2 py-1 text-xs text-slate-400">
                <span>${t("shell.language")}</span>
                <select id="language-select" class="bg-slate-950 text-cyan-300 focus:outline-none" aria-label="${t("shell.language")}">
                  <option value="en" ${getLanguage() === "en" ? "selected" : ""}>EN</option>
                  <option value="fr" ${getLanguage() === "fr" ? "selected" : ""}>FR</option>
                </select>
              </label>
              <button id="btn-help-tour" type="button" class="inline-flex h-8 w-8 items-center justify-center rounded border border-cyan-500/40 bg-cyan-500/10 font-mono text-sm font-semibold text-cyan-300 hover:bg-cyan-500/20" title="${t("shell.help")}" aria-label="${t("shell.help")}">?</button>
              <span class="text-xs uppercase tracking-[0.2em] text-slate-500" aria-hidden="true">${t("shell.operator")}</span>
              <span id="shell-user" class="text-sm text-cyan-300 font-mono" aria-label="${t("shell.user-label")}: ${this.activeUser}">${this.activeUser}</span>
            </div>
          </header>
          <main id="main-content" tabindex="-1" class="min-h-0 flex-1 overflow-y-auto p-8 opacity-0" aria-label="${t("shell.main-label")}">
            <section data-route-panel="dashboard" class="route-panel space-y-6">
              <div class="grid grid-cols-1 gap-6 md:grid-cols-3">
                <div class="rounded-xl border border-slate-800 bg-slate-900 p-6">
                  <h3 class="text-sm font-medium text-slate-400">${t("dashboard.wasm")}</h3>
                  <div class="mt-3 text-3xl font-light text-white">${t("dashboard.ready")}</div>
                </div>
                <div class="rounded-xl border border-slate-800 bg-slate-900 p-6">
                  <h3 class="text-sm font-medium text-slate-400">${t("dashboard.routing")}</h3>
                  <div class="mt-3 text-3xl font-light text-white">1</div>
                </div>
                <div class="rounded-xl border border-slate-800 bg-slate-900 p-6">
                  <h3 class="text-sm font-medium text-slate-400">${t("dashboard.identity")}</h3>
                  <div class="mt-3 text-3xl font-light text-white">${t("dashboard.ready")}</div>
                </div>
              </div>
              ${
                this.activeRole === "admin"
                  ? '<tachyon-iam mode="admin"></tachyon-iam>'
                  : ""
              }
            </section>
          </main>
        </div>
        <tachyon-guided-tour></tachyon-guided-tour>
        <tachyon-toast-manager></tachyon-toast-manager>
        <tachyon-bundle-conflict-modal></tachyon-bundle-conflict-modal>
      </section>
    `;

    this.root.querySelectorAll<HTMLButtonElement>("[data-route]").forEach((button) => {
      button.addEventListener("click", () => {
        const route = button.dataset.route ?? "dashboard";
        this.dispatchEvent(new CustomEvent("app:navigation", { bubbles: true, composed: true, detail: { route } }));
        if (window.location.hash.slice(1) === route) {
          this.onHashChange();
          return;
        }
        window.location.hash = route;
      });
    });
    this.root.getElementById("language-select")?.addEventListener("change", (event) => {
      setLanguage((event.target as HTMLSelectElement).value);
    });
    this.root.getElementById("btn-help-tour")?.addEventListener("click", () => {
      this.guidedTour()?.start();
    });
    this.root.getElementById("btn-seal-apply")?.addEventListener("click", () => {
      void this.sealAndApply();
    });
  }

  private updateNavigation(route: string): void {
    this.root.querySelectorAll<HTMLButtonElement>("[data-route]").forEach((button) => {
      const active = button.dataset.route === route;
      button.classList.toggle("bg-slate-800", active);
      button.classList.toggle("text-cyan-400", active);
      button.classList.toggle("text-slate-300", !active);
    });
  }

  private showRoute(route: string): void {
    this.activeRoute = this.normalizeRoute(route);
    const staticPanel = this.root.querySelector<HTMLElement>(`[data-route-panel="${this.activeRoute}"]`);
    this.root.querySelectorAll<HTMLElement>("[data-route-panel]").forEach((panel) => {
      panel.classList.toggle("hidden", panel !== staticPanel);
    });

    if (staticPanel) {
      return;
    }

    const routerView = this.root.getElementById("main-content");
    if (!routerView) {
      return;
    }

    const tagName = resolveComponentTag(this.activeRoute);
    const dynamicPanel = this.ensureDynamicPanel(routerView);
    dynamicPanel.classList.remove("hidden");
    if (!tagName) {
      dynamicPanel.replaceChildren();
      const warning = document.createElement("div");
      warning.className =
        "rounded-xl border border-amber-500/30 bg-amber-500/10 px-4 py-3 text-sm text-amber-200";
      warning.textContent = `Unknown route: ${route}`;
      dynamicPanel.appendChild(warning);
      return;
    }

    dynamicPanel.replaceChildren();
    dynamicPanel.appendChild(document.createElement(tagName));
  }

  private ensureDynamicPanel(routerView: HTMLElement): HTMLElement {
    let panel = this.root.querySelector<HTMLElement>('[data-route-panel="dynamic"]');
    if (!panel) {
      panel = document.createElement("section");
      panel.dataset.routePanel = "dynamic";
      panel.className = "route-panel";
      routerView.appendChild(panel);
    }
    return panel;
  }

  private guidedTour(): GuidedTourElement | null {
    return this.root.querySelector<GuidedTourElement>("tachyon-guided-tour");
  }

  private restoreStartedState(): void {
    const shell = this.root.getElementById("shell");
    const sidebar = this.root.getElementById("shell-sidebar");
    const header = this.root.getElementById("shell-header");
    const content = this.root.getElementById("main-content");
    const user = this.root.getElementById("shell-user");
    shell?.classList.remove("hidden");
    shell?.classList.add("flex");
    if (sidebar) {
      sidebar.style.opacity = "1";
    }
    if (header) {
      header.style.opacity = "1";
    }
    if (content) {
      content.style.opacity = "1";
    }
    if (user) {
      user.textContent = this.activeUser;
    }
    this.updateNavigation(this.activeRoute);
    this.showRoute(this.activeRoute);
  }

  private resolveHashRoute(): string {
    return this.normalizeRoute(window.location.hash.replace(/^#/, ""));
  }

  private normalizeRoute(route: string): string {
    const normalized = route.trim().replace(/^\//, "") || "dashboard";
    if (normalized === "dashboard") {
      return normalized;
    }
    return listComponentRoutes().some((entry) => entry.route === normalized) ? normalized : "dashboard";
  }

  private async sealAndApply(): Promise<void> {
    if (this.applyingSeal) {
      return;
    }
    this.applyingSeal = true;
    this.render();
    this.restoreStartedState();
    try {
      const result = await invoke<BundleApplyOutcome>("bundle_and_apply_manifest", {
        dependencies: [],
      });
      if (result.requiresResolution && result.conflicts.length > 0) {
        const modal = this.root.querySelector<BundleConflictModalElement>(
          "tachyon-bundle-conflict-modal",
        );
        modal?.open(result.conflicts);
        window.dispatchEvent(
          new CustomEvent("app:notify", {
            detail: { type: "error", message: result.message },
          }),
        );
        return;
      }
      this.requiresSeal = false;
      window.dispatchEvent(
        new CustomEvent("app:notify", {
          detail: {
            type: "success",
            message: `${result.message} config_version=${result.configVersion}`,
          },
        }),
      );
      const header = this.root.getElementById("shell-header");
      if (header) {
        void gsap.fromTo(
          header,
          { boxShadow: "0 0 0 rgba(34,211,238,0)" },
          {
            boxShadow: "0 0 28px rgba(34,211,238,0.25)",
            duration: 0.35,
            yoyo: true,
            repeat: 1,
          },
        );
      }
    } catch (error) {
      // Fallback: if the bundle endpoint is unreachable on legacy nodes, attempt
      // the classic seal-and-apply path so operators are not blocked.
      try {
        const fallback = await invoke<SealApplyOutcome>("seal_and_apply_manifest");
        this.requiresSeal = false;
        window.dispatchEvent(
          new CustomEvent("app:notify", {
            detail: {
              type: "success",
              message: `${fallback.message} config_version=${fallback.configVersion}`,
            },
          }),
        );
      } catch (fallbackError) {
        window.dispatchEvent(
          new CustomEvent("app:notify", {
            detail: {
              type: "error",
              message:
                error instanceof Error
                  ? error.message
                  : fallbackError instanceof Error
                    ? fallbackError.message
                    : String(fallbackError),
            },
          }),
        );
      }
    } finally {
      this.applyingSeal = false;
      this.render();
      this.restoreStartedState();
    }
  }
}

customElements.define("tachyon-app-shell", TachyonAppShell);

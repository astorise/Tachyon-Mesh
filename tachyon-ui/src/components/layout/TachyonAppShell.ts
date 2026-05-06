import gsap from "gsap";

import stylesheetText from "../../style.css?inline";

type AuthenticatedDetail = {
  user: string;
  role: string;
  token: string;
};

const appShellStylesheet = new CSSStyleSheet();
appShellStylesheet.replaceSync(stylesheetText);

export class TachyonAppShell extends HTMLElement {
  private readonly root: ShadowRoot;
  private readonly onAuthenticated = (event: Event) => {
    const detail = (event as CustomEvent<AuthenticatedDetail>).detail;
    void this.startTransition(detail);
  };

  constructor() {
    super();
    this.root = this.attachShadow({ mode: "open" });
    this.root.adoptedStyleSheets = [appShellStylesheet];
  }

  connectedCallback(): void {
    this.render();
    document.addEventListener("iam:authenticated", this.onAuthenticated);
  }

  disconnectedCallback(): void {
    document.removeEventListener("iam:authenticated", this.onAuthenticated);
  }

  async startTransition(userData: AuthenticatedDetail): Promise<void> {
    const authLayer = document.getElementById("auth-layer");
    const shell = this.root.getElementById("shell");
    const sidebar = this.root.getElementById("shell-sidebar");
    const header = this.root.getElementById("shell-header");
    const content = this.root.getElementById("router-view");
    const user = this.root.getElementById("shell-user");
    if (!shell || !sidebar || !header || !content || !user) {
      return;
    }

    user.textContent = userData.user;
    authLayer?.classList.add("hidden");
    this.classList.remove("hidden");
    shell.classList.remove("hidden");
    shell.classList.add("flex");

    const timeline = gsap.timeline();
    timeline
      .fromTo(sidebar, { x: -50, opacity: 0 }, { x: 0, opacity: 1, duration: 0.28, ease: "power2.out" })
      .fromTo(header, { y: -20, opacity: 0 }, { y: 0, opacity: 1, duration: 0.22 }, "-=0.12")
      .fromTo(content, { opacity: 0 }, { opacity: 1, duration: 0.22 }, "-=0.08");
    await timeline.then();
  }

  private render(): void {
    this.root.innerHTML = `
      <section id="shell" class="hidden fixed inset-0 z-30 h-screen w-screen bg-slate-950 text-slate-300">
        <aside id="shell-sidebar" class="w-64 bg-slate-900 border-r border-slate-800 flex flex-col opacity-0">
          <div class="h-16 flex items-center px-6 border-b border-slate-800">
            <div class="w-3 h-3 bg-cyan-400 rounded-full mr-3 shadow-[0_0_10px_rgba(34,211,238,0.8)]"></div>
            <h1 class="text-xl font-bold text-white tracking-wider">TACHYON<span class="text-cyan-400">MESH</span></h1>
          </div>
          <nav class="flex-1 p-4 space-y-2">
            <button data-route="dashboard" class="nav-link w-full text-left block px-4 py-2 rounded-md bg-slate-800 text-cyan-400 font-medium transition-colors">Dashboard</button>
            <button data-route="routing" class="nav-link w-full text-left block px-4 py-2 rounded-md text-slate-300 hover:bg-slate-800/50 transition-colors">Routing</button>
            <button data-route="topology" class="nav-link w-full text-left block px-4 py-2 rounded-md text-slate-300 hover:bg-slate-800/50 transition-colors">Mesh Topology</button>
            <button data-route="registry" class="nav-link w-full text-left block px-4 py-2 rounded-md text-slate-300 hover:bg-slate-800/50 transition-colors">Asset Registry</button>
          </nav>
          <div class="p-4 border-t border-slate-800 text-xs text-slate-500">v1.0.0-webcomponents</div>
        </aside>
        <div class="flex min-w-0 flex-1 flex-col">
          <header id="shell-header" class="h-16 border-b border-slate-800 flex items-center justify-between px-8 bg-slate-900/50 backdrop-blur-md opacity-0">
            <div>
              <h2 class="text-lg font-semibold text-white">Control Plane</h2>
              <p class="text-xs text-slate-500">Native Web Components shell</p>
            </div>
            <div class="flex items-center gap-3">
              <span class="text-xs uppercase tracking-[0.2em] text-slate-500">Operator</span>
              <span id="shell-user" class="text-sm text-cyan-300 font-mono">unknown</span>
            </div>
          </header>
          <main id="router-view" class="min-h-0 flex-1 overflow-y-auto p-8 opacity-0">
            <section data-route-panel="dashboard" class="route-panel space-y-6">
              <div class="grid grid-cols-1 gap-6 md:grid-cols-3">
                <div class="rounded-xl border border-slate-800 bg-slate-900 p-6">
                  <h3 class="text-sm font-medium text-slate-400">Wasm Engine</h3>
                  <div class="mt-3 text-3xl font-light text-white">Ready</div>
                </div>
                <div class="rounded-xl border border-slate-800 bg-slate-900 p-6">
                  <h3 class="text-sm font-medium text-slate-400">Routing</h3>
                  <div class="mt-3 text-3xl font-light text-white">1</div>
                </div>
                <div class="rounded-xl border border-slate-800 bg-slate-900 p-6">
                  <h3 class="text-sm font-medium text-slate-400">Identity</h3>
                  <div class="mt-3 text-3xl font-light text-white">Active</div>
                </div>
              </div>
            </section>
          </main>
        </div>
      </section>
    `;

    this.root.querySelectorAll<HTMLButtonElement>("[data-route]").forEach((button) => {
      button.addEventListener("click", () => {
        const route = button.dataset.route ?? "dashboard";
        this.dispatchEvent(new CustomEvent("app:navigation", { bubbles: true, composed: true, detail: { route } }));
        this.updateNavigation(route);
      });
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
}

customElements.define("tachyon-app-shell", TachyonAppShell);

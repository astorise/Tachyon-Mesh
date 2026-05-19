import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { resilientInvoke as invoke } from "../../utils/network";

type RegisteredSystem = {
  slug: string;
  crateName: string;
  version: string;
  description: string;
};

type DeployedSystem = {
  slug: string;
  catalogVersion: string;
  status: string;
  hostNodeCount: number;
  hasDrift: boolean;
  nodes: Array<{ nodeId: string; version: string }>;
};

export class TachyonSystemsPanel extends TachyonConfigDashboard {
  private registered: RegisteredSystem[] = [];
  private deployed = new Map<string, DeployedSystem>();
  private expanded: string | null = null;

  connectedCallback(): void {
    this.render();
    this.bindEvents();
    void this.load();
  }

  private async load(): Promise<void> {
    await this.withLoadingState(async () => {
      const [registered, deployed] = await Promise.all([
        invoke<RegisteredSystem[]>("list_registered_systems"),
        invoke<DeployedSystem[]>("list_deployed_systems"),
      ]);
      this.registered = registered;
      this.deployed = new Map(deployed.map((system) => [system.slug, system]));
      this.render();
      this.bindEvents();
    });
  }

  private render(): void {
    this.renderTemplate(`
      <section class="p-6 space-y-5 text-slate-300">
        <header>
          <h2 class="text-2xl font-bold text-slate-100">Systems</h2>
          <p class="text-xs font-mono text-slate-500">${this.registered.length} catalog entries</p>
        </header>
        <article class="overflow-hidden rounded-lg border border-slate-800 bg-slate-900">
          <table class="w-full text-left text-sm">
            <thead class="bg-slate-950/60 text-xs uppercase tracking-widest text-slate-500">
              <tr><th class="p-3">Slug</th><th class="p-3">Catalog</th><th class="p-3">Status</th><th class="p-3">Hosts</th></tr>
            </thead>
            <tbody>
              ${this.registered.map((system) => this.row(system)).join("")}
            </tbody>
          </table>
        </article>
      </section>
    `);
  }

  private bindEvents(): void {
    this.root.querySelectorAll<HTMLButtonElement>("[data-system-slug]").forEach((button) => {
      button.addEventListener("click", () => {
        this.expanded = this.expanded === button.dataset.systemSlug ? null : button.dataset.systemSlug ?? null;
        this.render();
        this.bindEvents();
      });
    });
  }

  private row(system: RegisteredSystem): string {
    const deployed = this.deployed.get(system.slug) ?? {
      slug: system.slug,
      catalogVersion: system.version,
      status: "not-deployed",
      hostNodeCount: 0,
      hasDrift: false,
      nodes: [],
    };
    const expanded = this.expanded === system.slug;
    return `
      <tr class="border-t border-slate-800">
        <td class="p-3"><button data-system-slug="${this.escape(system.slug)}" class="font-mono text-cyan-300 hover:text-cyan-100">${this.escape(system.slug)}</button></td>
        <td class="p-3 font-mono">${this.escape(system.version)}</td>
        <td class="p-3">${this.badge(deployed.status)}</td>
        <td class="p-3 font-mono">${deployed.hostNodeCount}</td>
      </tr>
      ${expanded ? `<tr class="border-t border-slate-800 bg-slate-950/40"><td colspan="4" class="p-3 text-xs text-slate-400">${this.expandedContent(deployed)}</td></tr>` : ""}
    `;
  }

  private expandedContent(system: DeployedSystem): string {
    if (system.nodes.length === 0) {
      return "not currently active";
    }
    return system.nodes.map((node) => `${this.escape(node.nodeId)} (${this.escape(node.version)})`).join(", ");
  }

  private badge(status: string): string {
    const color = status === "deployed" ? "text-cyan-200 border-cyan-500/40 bg-cyan-500/10" : status === "version-drift" ? "text-amber-200 border-amber-500/40 bg-amber-500/10" : "text-slate-300 border-slate-700 bg-slate-800";
    return `<span class="inline-flex rounded border px-2 py-1 text-xs ${color}">${this.escape(status)}</span>`;
  }

  private escape(value: string): string {
    return value.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
  }
}

customElements.define("tachyon-systems-panel", TachyonSystemsPanel);

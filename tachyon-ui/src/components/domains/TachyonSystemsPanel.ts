import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { resilientInvoke as invoke } from "../../utils/network";
import { t } from "../../utils/i18n";

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
  private stagedEnabled = new Set<string>();
  private stagedDisabled = new Set<string>();
  private expanded: string | null = null;
  private busy = new Set<string>();

  connectedCallback(): void {
    this.render();
    this.bindEvents();
    void this.load();
  }

  private async load(): Promise<void> {
    await this.withLoadingState(async () => {
      const [registered, deployed, stagedRoutes] = await Promise.all([
        invoke<RegisteredSystem[]>("list_registered_systems"),
        invoke<DeployedSystem[]>("list_deployed_systems"),
        invoke<Array<[string, boolean]>>("get_staged_system_routes").catch(() => [] as Array<[string, boolean]>),
      ]);
      this.registered = registered;
      this.deployed = new Map(deployed.map((s) => [s.slug, s]));
      this.stagedEnabled = new Set(stagedRoutes.filter(([, on]) => on).map(([slug]) => slug));
      this.stagedDisabled = new Set(stagedRoutes.filter(([, on]) => !on).map(([slug]) => slug));
      this.render();
      this.bindEvents();
    });
  }

  private render(): void {
    this.renderTemplate(`
      <section class="p-6 space-y-5 text-slate-300">
        <header class="flex items-end justify-between">
          <div>
            <h2 class="text-2xl font-bold text-slate-100">Systems</h2>
            <p class="text-xs font-mono text-slate-500">${this.registered.length} ${t("systems.catalog.entries")}</p>
          </div>
          ${this.stagedEnabled.size + this.stagedDisabled.size > 0 ? `
          <div class="flex items-center gap-3">
            <span class="text-xs text-amber-300 font-mono">${this.stagedEnabled.size + this.stagedDisabled.size} ${t("systems.staged.pending")}</span>
            <button id="btn-apply-systems" class="rounded border border-cyan-500/60 bg-cyan-600/20 px-3 py-1.5 text-xs font-bold text-cyan-200 hover:bg-cyan-600/30">
              ${t("systems.apply")}
            </button>
          </div>` : ""}
        </header>
        <article class="overflow-hidden rounded-lg border border-slate-800 bg-slate-900">
          <table class="w-full text-left text-sm">
            <thead class="bg-slate-950/60 text-xs uppercase tracking-widest text-slate-500">
              <tr>
                <th class="p-3">${t("systems.col.slug")}</th>
                <th class="p-3">${t("systems.col.version")}</th>
                <th class="p-3">${t("systems.col.status")}</th>
                <th class="p-3">${t("systems.col.hosts")}</th>
                <th class="p-3">${t("systems.col.action")}</th>
              </tr>
            </thead>
            <tbody id="systems-tbody">
              ${this.registered.map((system) => this.row(system)).join("")}
            </tbody>
          </table>
        </article>
        <div id="feedback-zone" class="hidden rounded-xl border border-slate-800 bg-slate-900 px-4 py-3 font-mono text-xs text-slate-400"></div>
      </section>
    `);
  }

  private bindEvents(): void {
    // Row expand
    this.root.querySelectorAll<HTMLButtonElement>("[data-system-slug]").forEach((button) => {
      button.addEventListener("click", () => {
        this.expanded = this.expanded === button.dataset.systemSlug ? null : (button.dataset.systemSlug ?? null);
        this.render();
        this.bindEvents();
      });
    });

    // Enable/Disable buttons
    this.root.querySelectorAll<HTMLButtonElement>("[data-toggle-slug]").forEach((button) => {
      button.addEventListener("click", async () => {
        const slug = button.dataset.toggleSlug ?? "";
        const version = button.dataset.version ?? "";
        const enable = button.dataset.action === "enable";
        if (!slug || this.busy.has(slug)) return;
        this.busy.add(slug);
        try {
          await invoke("toggle_system_route", { slug, version, enabled: enable });
          if (enable) {
            this.stagedEnabled.add(slug);
            this.stagedDisabled.delete(slug);
          } else {
            this.stagedDisabled.add(slug);
            this.stagedEnabled.delete(slug);
          }
        } catch (error) {
          this.showFeedback("error", error instanceof Error ? error.message : String(error));
        } finally {
          this.busy.delete(slug);
          this.render();
          this.bindEvents();
        }
      });
    });

    // Apply (seal + apply manifest)
    this.root.getElementById("btn-apply-systems")?.addEventListener("click", async () => {
      try {
        const result = await invoke<{ success: boolean; message: string }>("seal_and_apply_manifest");
        this.showFeedback(result.success ? "success" : "error", result.message);
        this.stagedEnabled.clear();
        this.stagedDisabled.clear();
        void this.load();
      } catch (error) {
        this.showFeedback("error", error instanceof Error ? error.message : String(error));
      }
    });
  }

  private row(system: RegisteredSystem): string {
    const deployed = this.deployed.get(system.slug) ?? {
      slug: system.slug, catalogVersion: system.version,
      status: "not-deployed", hostNodeCount: 0, hasDrift: false, nodes: [],
    };
    const isStagedOn = this.stagedEnabled.has(system.slug);
    const isStagedOff = this.stagedDisabled.has(system.slug);
    const isBusy = this.busy.has(system.slug);
    const expanded = this.expanded === system.slug;

    const effectiveStatus = isStagedOn
      ? "staged-enable"
      : isStagedOff
        ? "staged-disable"
        : deployed.status;

    const canDisable = deployed.status !== "not-deployed" || isStagedOn;
    const canEnable = deployed.status === "not-deployed" && !isStagedOn;

    const actionBtn = isBusy
      ? `<span class="inline-block h-3 w-3 animate-spin rounded-full border-2 border-cyan-300 border-t-transparent"></span>`
      : canEnable
        ? `<button data-toggle-slug="${this.escape(system.slug)}" data-version="${this.escape(system.version)}" data-action="enable"
            class="rounded border border-emerald-500/40 bg-emerald-500/10 px-2 py-1 text-xs text-emerald-300 hover:bg-emerald-500/20">
            ${t("systems.action.enable")}
          </button>`
        : canDisable
          ? `<button data-toggle-slug="${this.escape(system.slug)}" data-version="${this.escape(system.version)}" data-action="disable"
              class="rounded border border-red-500/40 bg-red-500/10 px-2 py-1 text-xs text-red-300 hover:bg-red-500/20">
              ${t("systems.action.disable")}
            </button>`
          : `<span class="text-slate-600 text-xs">—</span>`;

    return `
      <tr class="border-t border-slate-800 hover:bg-slate-800/30 transition-colors">
        <td class="p-3"><button data-system-slug="${this.escape(system.slug)}" class="font-mono text-cyan-300 hover:text-cyan-100 text-left">${this.escape(system.slug)}</button></td>
        <td class="p-3 font-mono text-slate-400">${this.escape(system.version)}</td>
        <td class="p-3">${this.badge(effectiveStatus)}</td>
        <td class="p-3 font-mono text-slate-400">${deployed.hostNodeCount}</td>
        <td class="p-3">${actionBtn}</td>
      </tr>
      ${expanded ? `<tr class="border-t border-slate-800 bg-slate-950/40">
        <td colspan="5" class="p-3 text-xs text-slate-400">${this.expandedContent(system, deployed)}</td>
      </tr>` : ""}
    `;
  }

  private expandedContent(system: RegisteredSystem, deployed: DeployedSystem): string {
    const parts: string[] = [];
    if (system.description) parts.push(`<p class="mb-2 text-slate-300">${this.escape(system.description)}</p>`);
    if (deployed.nodes.length > 0) {
      const nodeList = deployed.nodes
        .map((n) => `<span class="mr-2 font-mono">${this.escape(n.nodeId)} <span class="text-slate-500">(${this.escape(n.version)})</span></span>`)
        .join("");
      parts.push(`<div>${nodeList}</div>`);
    } else {
      parts.push(`<span>${t("systems.no-nodes")}</span>`);
    }
    return parts.join("");
  }

  private badge(status: string): string {
    const styles: Record<string, string> = {
      "deployed":       "text-emerald-200 border-emerald-500/40 bg-emerald-500/10",
      "version-drift":  "text-amber-200  border-amber-500/40  bg-amber-500/10",
      "staged-enable":  "text-amber-300  border-amber-500/60  bg-amber-500/15",
      "staged-disable": "text-red-300    border-red-500/40    bg-red-500/10",
      "not-deployed":   "text-slate-400  border-slate-700     bg-slate-800/60",
    };
    const label: Record<string, string> = {
      "deployed":       t("systems.status.deployed"),
      "version-drift":  t("systems.status.version-drift"),
      "staged-enable":  t("systems.status.staged-enable"),
      "staged-disable": t("systems.status.staged-disable"),
      "not-deployed":   t("systems.status.not-deployed"),
    };
    const cls = styles[status] ?? styles["not-deployed"];
    const text = label[status] ?? this.escape(status);
    return `<span class="inline-flex rounded border px-2 py-0.5 text-xs font-mono ${cls}">${text}</span>`;
  }

  private escape(value: string): string {
    return value.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
  }
}

customElements.define("tachyon-systems-panel", TachyonSystemsPanel);

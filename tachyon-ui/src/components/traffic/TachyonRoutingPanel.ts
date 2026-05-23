import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { el } from "../../utils/dom-safe";
import { applyAndSeal, resilientInvoke as invoke } from "../../utils/network";
import { t } from "../../utils/i18n";
import "./TachyonVolumesPanel";
import "./TachyonConcurrencyPolicyPanel";
import "../routing/TachyonScopesPanel";

type MeshRouteSummary = {
  name: string;
  path: string;
  role: string;
  targetCount: number;
  requiresTee: boolean;
  encryptedVolumeCount: number;
  allowAllScopes: boolean;
};

type MeshGraphSnapshot = {
  source: string;
  status: string;
  routes: MeshRouteSummary[];
  batchTargets: string[];
};

export class TachyonRoutingPanel extends TachyonConfigDashboard {
  private snapshot: MeshGraphSnapshot | null = null;
  private expandedRoute: string | null = null;

  async connectedCallback(): Promise<void> {
    this.render();
    this.bindForm();
    this.animateEntrance();
    try {
      this.snapshot = await invoke<MeshGraphSnapshot>("get_mesh_graph");
    } catch {
      this.snapshot = null;
    }
    this.render();
    this.bindForm();
  }

  private bindForm(): void {
    this.root.querySelector("form")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.applyRoute();
    });
  }

  private render(): void {
    this.renderTemplate(`
      <section class="p-6 space-y-6 text-slate-300">
        <div data-stagger-panel class="border-l-4 border-cyan-500 pl-4">
          <h2 class="text-2xl font-bold text-slate-100">Gateway Routing</h2>
          <p class="text-slate-400 text-sm font-mono">Domain: routing.wit</p>
        </div>

        <article data-stagger-panel class="rounded-lg border border-slate-800 bg-slate-900/70 p-5">
          <h3 class="mb-3 text-sm font-semibold uppercase tracking-widest text-cyan-300">${t("routing.preview.title")}</h3>
          ${this.renderSnapshot()}
        </article>

        <form class="space-y-6">
          <div data-stagger-panel class="grid grid-cols-1 md:grid-cols-2 gap-6 bg-slate-800/40 p-6 rounded-lg border border-slate-700">
            <div class="space-y-4">
              <label class="block text-xs uppercase tracking-widest text-cyan-500">Inbound Path
                <input id="route-path" type="text" value="/api/v1" class="mt-1 w-full bg-slate-900 border border-slate-600 p-2 rounded text-slate-200 outline-none focus:border-cyan-400 transition-colors">
              </label>
              <label class="block text-xs uppercase tracking-widest text-cyan-500">Target Workload
                <input id="route-target" type="text" value="inference-service" class="mt-1 w-full bg-slate-900 border border-slate-600 p-2 rounded text-slate-200 outline-none focus:border-cyan-400 transition-colors">
              </label>
            </div>
            <div class="bg-slate-900/50 p-4 rounded border border-slate-800 flex items-center justify-center">
              <p class="text-[10px] text-slate-500 italic">L7 Load Balancing: Weighted Round Robin (Auto)</p>
            </div>
          </div>

          <button id="apply-btn" class="bg-cyan-600 hover:bg-cyan-500 text-slate-900 font-black py-3 px-8 rounded-sm uppercase tracking-tighter transition-all">
            Deploy Route
          </button>
        </form>

        <div id="feedback-zone" data-stagger-panel class="mt-4 rounded-xl border border-slate-800 bg-slate-900 px-4 py-3 font-mono text-xs text-slate-400">Awaiting route deployment.</div>
      </section>
    `);
    this.populateSnapshotRows();
  }

  private async applyRoute(): Promise<void> {
    try {
      const response = await applyAndSeal("config-routing", this.buildPayload());
      this.showFeedback(response.success ? "success" : "error", response.message);
    } catch (error) {
      this.showFeedback("error", error instanceof Error ? error.message : String(error));
    }
  }

  private buildPayload(): unknown {
    const path = this.value("route-path", "/api/v1");
    const target = this.value("route-target", "inference-service");
    return {
      api_version: "routing.tachyon.io/v1alpha1",
      kind: "TrafficConfiguration",
      metadata: {
        name: "edge-main-routing",
        environment: "production",
      },
      spec: {
        gateways: [
          {
            name: "public-https",
            protocol: "HTTPS",
            bind_address: "0.0.0.0:443",
          },
        ],
        routes: [
          {
            name: "operator-route",
            gateway_refs: ["public-https"],
            type: "HTTP",
            rules: [
              {
                match: { path: { prefix: path } },
                target,
              },
            ],
          },
        ],
      },
    };
  }

  private renderSnapshot(): string {
    if (!this.snapshot || this.snapshot.routes.length === 0) {
      return `<p class="text-xs text-slate-500">${t("routing.preview.empty")}</p>`;
    }
    // Static structure — rows populated via DOM API in populateSnapshotRows().
    return `
      <table class="w-full text-xs">
        <thead class="text-slate-500 uppercase tracking-widest">
          <tr><th class="text-left pb-2 pr-4">${t("routing.preview.column.name")}</th><th class="text-left pb-2 pr-4">${t("routing.preview.column.path")}</th><th class="text-left pb-2 pr-4">${t("routing.preview.column.targets")}</th><th class="text-left pb-2 pr-4">${t("routing.preview.column.tee")}</th><th class="text-left pb-2 pr-2">Scopes</th><th></th></tr>
        </thead>
        <tbody id="routing-snapshot-rows"></tbody>
      </table>
    `;
  }

  private populateSnapshotRows(): void {
    const tbody = this.root.getElementById("routing-snapshot-rows");
    if (!tbody || !this.snapshot) return;

    const rows: Element[] = [];
    for (const route of this.snapshot.routes) {
      const isExpanded = this.expandedRoute === route.path;
      const scopeBadge = route.allowAllScopes
        ? el("span", {
            class: "rounded-full bg-amber-500/20 border border-amber-500/40 px-1.5 py-0.5 text-[9px] font-semibold text-amber-300 uppercase tracking-widest cursor-help",
            title: "This deployment grants all WIT imports. Click to configure scopes.",
          }, "allow-all")
        : el("span", {
            class: "rounded-full bg-emerald-500/20 border border-emerald-500/40 px-1.5 py-0.5 text-[9px] font-semibold text-emerald-300 uppercase tracking-widest",
          }, "scoped");
      const tr = el("tr", { class: "cursor-pointer hover:bg-slate-800/40", "data-route-path": route.path },
        el("td", { class: "py-1 pr-4 text-cyan-300" }, route.name),
        el("td", { class: "py-1 pr-4 font-mono text-slate-300" }, route.path),
        el("td", { class: "py-1 pr-4 text-slate-300" }, String(route.targetCount)),
        el("td", { class: "py-1 text-slate-300" }, route.requiresTee ? "yes" : "no"),
        el("td", { class: "py-1 pr-2" }, scopeBadge),
        el("td", { class: "py-1 text-slate-400 text-xs" }, isExpanded ? "▲" : "▼"),
      );
      tr.addEventListener("click", () => {
        this.expandedRoute = isExpanded ? null : route.path;
        this.populateSnapshotRows();
      });
      rows.push(tr);

      if (isExpanded) {
        const wrapper = document.createElement("div");
        wrapper.className = "space-y-2";
        const concPanel = document.createElement("tachyon-concurrency-policy-panel") as HTMLElement;
        concPanel.setAttribute("route-path", route.path);
        const volPanel = document.createElement("tachyon-volumes-panel") as HTMLElement;
        volPanel.setAttribute("route-path", route.path);
        const scopesPanel = document.createElement("tachyon-scopes-panel") as HTMLElement;
        scopesPanel.setAttribute("route-path", route.path);
        wrapper.appendChild(concPanel);
        wrapper.appendChild(volPanel);
        wrapper.appendChild(scopesPanel);
        const detailRow = el("tr", {},
          el("td", { colspan: "6", class: "pb-2" }, wrapper),
        );
        rows.push(detailRow);
      }
    }

    tbody.replaceChildren(...rows);
  }

  private value(id: string, fallback: string): string {
    const value = (this.root.getElementById(id) as HTMLInputElement | null)?.value.trim();
    return value ? value : fallback;
  }
}

customElements.define("tachyon-routing-panel", TachyonRoutingPanel);

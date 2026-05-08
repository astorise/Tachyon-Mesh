import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { resilientInvoke as invoke } from "../../utils/network";
import { t } from "../../utils/i18n";

type ApplyConfigurationResponse = {
  success: boolean;
  message: string;
};

type MeshRouteSummary = {
  name: string;
  path: string;
  role: string;
  targetCount: number;
  requiresTee: boolean;
  encryptedVolumeCount: number;
};

type MeshGraphSnapshot = {
  source: string;
  status: string;
  routes: MeshRouteSummary[];
  batchTargets: string[];
};

export class TachyonRoutingPanel extends TachyonConfigDashboard {
  private snapshot: MeshGraphSnapshot | null = null;

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
  }

  private async applyRoute(): Promise<void> {
    try {
      const response = await invoke<ApplyConfigurationResponse>("apply_configuration", {
        domain: "config-routing",
        payload: this.buildPayload(),
      });
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
    const rows = this.snapshot.routes
      .map(
        (route) =>
          `<tr><td class="py-1 pr-4 text-cyan-300">${this.escape(route.name)}</td><td class="py-1 pr-4 font-mono text-slate-300">${this.escape(route.path)}</td><td class="py-1 pr-4 text-slate-300">${route.targetCount}</td><td class="py-1 text-slate-300">${route.requiresTee ? "yes" : "no"}</td></tr>`,
      )
      .join("");
    return `
      <table class="w-full text-xs">
        <thead class="text-slate-500 uppercase tracking-widest">
          <tr><th class="text-left pb-2 pr-4">${t("routing.preview.column.name")}</th><th class="text-left pb-2 pr-4">${t("routing.preview.column.path")}</th><th class="text-left pb-2 pr-4">${t("routing.preview.column.targets")}</th><th class="text-left pb-2">${t("routing.preview.column.tee")}</th></tr>
        </thead>
        <tbody>${rows}</tbody>
      </table>
    `;
  }

  private value(id: string, fallback: string): string {
    const value = (this.root.getElementById(id) as HTMLInputElement | null)?.value.trim();
    return value ? value : fallback;
  }

  private escape(value: string): string {
    return value
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#039;");
  }
}

customElements.define("tachyon-routing-panel", TachyonRoutingPanel);

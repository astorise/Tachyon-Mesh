import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { resilientInvoke as invoke } from "../../utils/network";

type ApplyConfigurationResponse = {
  success: boolean;
  message: string;
};

export class TachyonRoutingPanel extends TachyonConfigDashboard {
  connectedCallback(): void {
    this.render();
    this.root.querySelector("form")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.applyRoute();
    });
    this.animateEntrance();
  }

  private render(): void {
    this.renderTemplate(`
      <section class="p-6 space-y-6 text-slate-300">
        <div data-stagger-panel class="border-l-4 border-cyan-500 pl-4">
          <h2 class="text-2xl font-bold text-slate-100">Gateway Routing</h2>
          <p class="text-slate-400 text-sm font-mono">Domain: routing.wit</p>
        </div>

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

  private value(id: string, fallback: string): string {
    const value = (this.root.getElementById(id) as HTMLInputElement | null)?.value.trim();
    return value ? value : fallback;
  }
}

customElements.define("tachyon-routing-panel", TachyonRoutingPanel);

import stylesheetText from "../../style.css?inline";
import { resilientInvoke as invoke } from "../../utils/network";

type ApplyConfigurationResponse = {
  success: boolean;
  message: string;
};

const routingDashboardStylesheet = new CSSStyleSheet();
routingDashboardStylesheet.replaceSync(stylesheetText);

export class TachyonRoutingDashboard extends HTMLElement {
  private readonly root: ShadowRoot;

  constructor() {
    super();
    this.root = this.attachShadow({ mode: "open" });
    this.root.adoptedStyleSheets = [routingDashboardStylesheet];
  }

  connectedCallback(): void {
    this.render();
    this.root.querySelector("form")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.applyConfiguration();
    });
  }

  private render(): void {
    this.root.innerHTML = `
      <section class="space-y-6 text-slate-300">
        <div class="flex items-center justify-between gap-4">
          <div>
            <h2 class="text-2xl font-bold text-white">Routing & Gateways</h2>
            <p class="text-sm text-slate-500">Apply a strict L4/L7 routing payload through the Tauri Serde validator.</p>
          </div>
          <span class="rounded-full border border-cyan-500/30 bg-cyan-500/10 px-3 py-1 text-xs font-medium text-cyan-300">config-routing</span>
        </div>
        <form class="grid gap-6 lg:grid-cols-3">
          <div class="rounded-lg border border-slate-800 bg-slate-900 p-4">
            <h3 class="mb-4 text-lg font-semibold text-slate-200">L4 Gateway</h3>
            <label class="block text-xs text-slate-500">Gateway Name
              <input id="gateway-name" type="text" value="public-https" class="mt-1 w-full rounded border border-slate-800 bg-slate-950 px-3 py-2 text-sm text-slate-200 focus:border-cyan-500 focus:outline-none" />
            </label>
            <label class="mt-3 block text-xs text-slate-500">Protocol
              <select id="gateway-protocol" class="mt-1 w-full rounded border border-slate-800 bg-slate-950 px-3 py-2 text-sm text-slate-200 focus:border-cyan-500 focus:outline-none">
                <option value="HTTPS">HTTPS</option>
                <option value="HTTP">HTTP</option>
                <option value="TCP">TCP</option>
              </select>
            </label>
            <label class="mt-3 block text-xs text-slate-500">Bind Address
              <input id="gateway-bind" type="text" value="0.0.0.0:443" class="mt-1 w-full rounded border border-slate-800 bg-slate-950 px-3 py-2 font-mono text-sm text-slate-200 focus:border-cyan-500 focus:outline-none" />
            </label>
          </div>
          <div class="rounded-lg border border-slate-800 bg-slate-900 p-4 lg:col-span-2">
            <h3 class="mb-4 text-lg font-semibold text-slate-200">L7 Route Rule</h3>
            <div class="grid gap-4 md:grid-cols-2">
              <label class="block text-xs text-slate-500">Route Name
                <input id="route-name" type="text" value="api-v1-routing" class="mt-1 w-full rounded border border-slate-800 bg-slate-950 px-3 py-2 text-sm text-slate-200 focus:border-cyan-500 focus:outline-none" />
              </label>
              <label class="block text-xs text-slate-500">Path Prefix
                <input id="route-path" type="text" value="/v1/inference" class="mt-1 w-full rounded border border-slate-800 bg-slate-950 px-3 py-2 font-mono text-sm text-emerald-400 focus:border-cyan-500 focus:outline-none" />
              </label>
            </div>
            <label class="mt-3 block text-xs text-slate-500">Target Workload
              <input id="route-target" type="text" value="ai-inference-wasm" class="mt-1 w-full rounded border border-slate-800 bg-slate-950 px-3 py-2 text-sm text-slate-200 focus:border-cyan-500 focus:outline-none" />
            </label>
            <button class="mt-5 rounded-lg bg-cyan-600 px-4 py-2.5 text-sm font-semibold text-white shadow-[0_0_15px_rgba(34,211,238,0.18)] transition-all hover:bg-cyan-500">Deploy Configuration</button>
          </div>
        </form>
        <div id="feedback-zone" class="rounded-xl border border-slate-800 bg-slate-900 px-4 py-3 font-mono text-xs text-slate-400">Awaiting configuration.</div>
      </section>
    `;
  }

  private async applyConfiguration(): Promise<void> {
    try {
      const response = await invoke<ApplyConfigurationResponse>("apply_configuration", {
        domain: "config-routing",
        payload: this.buildPayload(),
      });
      this.showFeedback(response.success ? "success" : "error", response.message);
      this.dispatchEvent(
        new CustomEvent(response.success ? "config:applied" : "config:error", {
          bubbles: true,
          composed: true,
          detail: response.success
            ? { domain: "routing", status: "success" }
            : { domain: "routing", message: response.message },
        }),
      );
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      this.showFeedback("error", message);
      this.dispatchEvent(new CustomEvent("config:error", { bubbles: true, composed: true, detail: { domain: "routing", message } }));
    }
  }

  private buildPayload(): unknown {
    const gatewayName = this.value("gateway-name", "public-https");
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
            name: gatewayName,
            protocol: this.value("gateway-protocol", "HTTPS"),
            bind_address: this.value("gateway-bind", "0.0.0.0:443"),
          },
        ],
        routes: [
          {
            name: this.value("route-name", "api-v1-routing"),
            gateway_refs: [gatewayName],
            type: "HTTP",
            rules: [
              {
                match: { path: { prefix: this.value("route-path", "/") } },
                target: this.value("route-target", "ai-inference-wasm"),
              },
            ],
          },
        ],
      },
    };
  }

  private value(id: string, fallback: string): string {
    const value = (this.root.getElementById(id) as HTMLInputElement | HTMLSelectElement | null)?.value.trim();
    return value ? value : fallback;
  }

  private showFeedback(kind: "success" | "error", message: string): void {
    const zone = this.root.getElementById("feedback-zone");
    if (!zone) {
      return;
    }
    zone.textContent = message;
    zone.classList.toggle("border-emerald-500/30", kind === "success");
    zone.classList.toggle("text-emerald-300", kind === "success");
    zone.classList.toggle("border-red-500/30", kind === "error");
    zone.classList.toggle("text-red-300", kind === "error");
  }
}

customElements.define("tachyon-routing-dashboard", TachyonRoutingDashboard);

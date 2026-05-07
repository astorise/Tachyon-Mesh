import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { resilientInvoke as invoke } from "../../utils/network";

type ApplyConfigurationResponse = {
  success: boolean;
  message: string;
};

export class TachyonFleetPanel extends TachyonConfigDashboard {
  connectedCallback(): void {
    this.render();
    this.root.querySelector("form")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.applyFleetConfig();
    });
    this.animateEntrance();
  }

  private render(): void {
    this.renderTemplate(`
      <section class="p-6 space-y-6 text-slate-300">
        <header data-stagger-panel class="border-l-4 border-cyan-500 pl-4">
          <h2 class="text-2xl font-bold text-slate-100">Fleet Placement</h2>
          <p class="text-sm font-mono text-slate-400">Domain: config-fleet / node selectors</p>
        </header>

        <form class="grid grid-cols-1 gap-6 rounded-lg border border-slate-700 bg-slate-800/40 p-6 md:grid-cols-2">
          <label data-stagger-panel class="block text-xs uppercase tracking-widest text-cyan-500">Node Selector Tags
            <input id="selector-tags" type="text" placeholder="region=eu-west,gpu=true" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm text-slate-200 outline-none transition-colors focus:border-cyan-400">
          </label>

          <label data-stagger-panel class="block text-xs uppercase tracking-widest text-cyan-500">Node Profile
            <select id="node-profile" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm text-slate-200 outline-none transition-colors focus:border-cyan-400">
              <option value="edge-light">Edge Light</option>
              <option value="core-heavy">Core Heavy</option>
            </select>
          </label>

          <button data-stagger-panel class="border border-cyan-500 px-6 py-3 font-bold text-cyan-500 transition-colors hover:bg-cyan-500 hover:text-slate-950 md:col-span-2 md:w-fit">
            Apply Fleet Policy
          </button>
        </form>

        <div id="feedback-zone" data-stagger-panel class="rounded-xl border border-slate-800 bg-slate-900 px-4 py-3 font-mono text-xs text-slate-400">Awaiting fleet placement policy.</div>
      </section>
    `);
  }

  private async applyFleetConfig(): Promise<void> {
    try {
      const response = await invoke<ApplyConfigurationResponse>("apply_configuration", {
        domain: "fleet",
        payload: {
          selector_tags: this.value("selector-tags", ""),
          node_profile: this.value("node-profile", "edge-light"),
        },
      });
      this.showFeedback(response.success ? "success" : "error", response.message);
    } catch (error) {
      this.showFeedback("error", error instanceof Error ? error.message : String(error));
    }
  }

  private value(id: string, fallback: string): string {
    const value = (this.root.getElementById(id) as HTMLInputElement | HTMLSelectElement | null)?.value.trim();
    return value ? value : fallback;
  }
}

customElements.define("tachyon-fleet-panel", TachyonFleetPanel);

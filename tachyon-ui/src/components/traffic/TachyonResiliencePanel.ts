import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { resilientInvoke as invoke } from "../../utils/network";

type ApplyConfigurationResponse = {
  success: boolean;
  message: string;
};

export class TachyonResiliencePanel extends TachyonConfigDashboard {
  connectedCallback(): void {
    this.render();
    this.root.querySelector("form")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.applyPolicy();
    });
    this.animateEntrance();
  }

  private render(): void {
    this.renderTemplate(`
      <section class="p-6 space-y-6 text-slate-300">
        <div data-stagger-panel class="border-l-4 border-cyan-500 pl-4">
          <h2 class="text-2xl font-bold text-slate-100">L7 Resilience</h2>
          <p class="text-slate-400 text-sm font-mono">Domain: config-resilience</p>
        </div>

        <form class="space-y-6">
          <div data-stagger-panel class="grid grid-cols-1 md:grid-cols-3 gap-4 bg-slate-800/40 p-6 rounded-lg border border-slate-700">
            <label class="block text-xs uppercase tracking-widest text-cyan-500">Timeout (ms)
              <input id="timeout-ms" type="number" min="1" value="1500" class="mt-1 w-full bg-slate-900 border border-slate-600 p-2 rounded text-slate-200 outline-none focus:border-cyan-400 transition-colors">
            </label>
            <label class="block text-xs uppercase tracking-widest text-cyan-500">Retry Count
              <input id="retry-count" type="number" min="0" value="2" class="mt-1 w-full bg-slate-900 border border-slate-600 p-2 rounded text-slate-200 outline-none focus:border-cyan-400 transition-colors">
            </label>
            <label class="block text-xs uppercase tracking-widest text-cyan-500">Circuit Breaker Threshold
              <input id="circuit-breaker-threshold" type="number" min="1" value="5" class="mt-1 w-full bg-slate-900 border border-slate-600 p-2 rounded text-slate-200 outline-none focus:border-cyan-400 transition-colors">
            </label>
          </div>

          <button id="apply-btn" class="bg-cyan-600 hover:bg-cyan-500 text-slate-900 font-black py-3 px-8 rounded-sm uppercase tracking-tighter transition-all">
            Apply Resilience
          </button>
        </form>

        <div id="feedback-zone" data-stagger-panel class="mt-4 rounded-xl border border-slate-800 bg-slate-900 px-4 py-3 font-mono text-xs text-slate-400">Awaiting resilience policy.</div>
      </section>
    `);
  }

  private async applyPolicy(): Promise<void> {
    try {
      const response = await invoke<ApplyConfigurationResponse>("apply_configuration", {
        domain: "config-resilience",
        payload: {
          timeout_ms: this.numberValue("timeout-ms", 1500),
          retry_count: this.numberValue("retry-count", 2),
          circuit_breaker_threshold: this.numberValue("circuit-breaker-threshold", 5),
        },
      });
      this.showFeedback(response.success ? "success" : "error", response.message);
    } catch (error) {
      this.showFeedback("error", error instanceof Error ? error.message : String(error));
    }
  }

  private numberValue(id: string, fallback: number): number {
    const input = this.root.getElementById(id) as HTMLInputElement | null;
    if (!input) {
      return fallback;
    }
    const raw = input.valueAsNumber;
    return Number.isFinite(raw) ? raw : fallback;
  }
}

customElements.define("tachyon-resilience-panel", TachyonResiliencePanel);

import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { resilientInvoke as invoke } from "../../utils/network";

type ApplyConfigurationResponse = {
  success: boolean;
  message: string;
};

export class TachyonSupplyChainPanel extends TachyonConfigDashboard {
  connectedCallback(): void {
    this.render();
    this.root.querySelector("form")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.applySupplyChainConfig();
    });
    this.animateEntrance();
  }

  private render(): void {
    this.renderTemplate(`
      <section class="p-6 space-y-6 text-slate-300">
        <header data-stagger-panel class="border-l-4 border-cyan-500 pl-4">
          <h2 class="text-2xl font-bold text-slate-100">Supply Chain</h2>
          <p class="text-sm font-mono text-slate-400">Domain: config-assets / air-gapped pushes</p>
        </header>

        <form class="space-y-6 rounded-lg border border-slate-700 bg-slate-800/40 p-6">
          <label data-stagger-panel class="block text-xs uppercase tracking-widest text-cyan-500">Asset Signature Key
            <input id="signature-key" type="text" placeholder="sha256:..." class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 font-mono text-sm text-slate-200 outline-none transition-colors focus:border-cyan-400">
          </label>

          <label data-stagger-panel class="flex items-center justify-between rounded border border-slate-700 bg-slate-900/60 px-4 py-3 text-xs uppercase tracking-widest text-cyan-500">
            <span>Air-Gapped Mode Enabled</span>
            <input id="air-gapped" type="checkbox" class="h-5 w-5 accent-cyan-500">
          </label>

          <button data-stagger-panel class="border border-cyan-500 px-6 py-3 font-bold text-cyan-500 transition-colors hover:bg-cyan-500 hover:text-slate-950">
            Apply Supply Chain Policy
          </button>
        </form>

        <div id="feedback-zone" data-stagger-panel class="rounded-xl border border-slate-800 bg-slate-900 px-4 py-3 font-mono text-xs text-slate-400">Awaiting supply chain policy.</div>
      </section>
    `);
  }

  private async applySupplyChainConfig(): Promise<void> {
    try {
      const response = await invoke<ApplyConfigurationResponse>("apply_configuration", {
        domain: "supply_chain",
        payload: {
          signature_key: this.value("signature-key", ""),
          air_gapped: (this.root.getElementById("air-gapped") as HTMLInputElement | null)?.checked ?? false,
        },
      });
      this.showFeedback(response.success ? "success" : "error", response.message);
    } catch (error) {
      this.showFeedback("error", error instanceof Error ? error.message : String(error));
    }
  }

  private value(id: string, fallback: string): string {
    const value = (this.root.getElementById(id) as HTMLInputElement | null)?.value.trim();
    return value ? value : fallback;
  }
}

customElements.define("tachyon-supply-chain-panel", TachyonSupplyChainPanel);

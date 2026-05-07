import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { resilientInvoke as invoke } from "../../utils/network";

type ApplyConfigurationResponse = {
  success: boolean;
  message: string;
};

export class TachyonHardwarePanel extends TachyonConfigDashboard {
  connectedCallback(): void {
    this.render();
    this.root.querySelector("form")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.applyHardwarePolicy();
    });
    this.animateEntrance();
  }

  private render(): void {
    this.renderTemplate(`
      <section class="p-6 space-y-6 text-slate-300">
        <div data-stagger-panel class="border-l-4 border-cyan-500 pl-4">
          <h2 class="text-2xl font-bold text-slate-100">Hardware Acceleration</h2>
          <p class="text-sm font-mono text-slate-400">Domain: config-ai / accelerator policy</p>
        </div>

        <form class="space-y-6">
          <div data-stagger-panel class="grid grid-cols-1 gap-6 rounded-lg border border-slate-700 bg-slate-800/40 p-6 md:grid-cols-2">
            <label class="block text-xs uppercase tracking-widest text-cyan-500">Accelerator
              <select id="accelerator" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm text-slate-200 outline-none transition-colors focus:border-cyan-400">
                <option value="npu">NPU</option>
                <option value="tpu">TPU</option>
                <option value="gpu">GPU</option>
              </select>
            </label>
            <label class="flex min-h-20 items-center justify-between rounded border border-slate-800 bg-slate-900/50 px-4 text-xs uppercase tracking-widest text-cyan-500">
              <span>eBPF XDP Offload</span>
              <input id="xdp-offload" type="checkbox" checked class="h-5 w-5 accent-cyan-500">
            </label>
          </div>

          <button id="apply-hardware" class="bg-cyan-600 px-8 py-3 font-black uppercase tracking-tight text-slate-950 transition-colors hover:bg-cyan-500">
            Apply Hardware Policy
          </button>
        </form>

        <div id="feedback-zone" data-stagger-panel class="mt-4 rounded-xl border border-slate-800 bg-slate-900 px-4 py-3 font-mono text-xs text-slate-400">Awaiting accelerator policy.</div>
      </section>
    `);
  }

  private async applyHardwarePolicy(): Promise<void> {
    try {
      const response = await invoke<ApplyConfigurationResponse>("apply_configuration", {
        domain: "config-ai",
        payload: {
          lora_mode: "dynamic",
          kv_cache_size: 32,
          tde_key: "hardware-policy-validation",
          accelerator: this.value("accelerator", "npu"),
          xdp_offload: (this.root.getElementById("xdp-offload") as HTMLInputElement | null)?.checked ?? true,
        },
      });
      this.showFeedback(response.success ? "success" : "error", response.message);
    } catch (error) {
      this.showFeedback("error", error instanceof Error ? error.message : String(error));
    }
  }

  private value(id: string, fallback: string): string {
    const value = (this.root.getElementById(id) as HTMLSelectElement | null)?.value.trim();
    return value ? value : fallback;
  }
}

customElements.define("tachyon-hardware-panel", TachyonHardwarePanel);

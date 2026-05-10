import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { resilientInvoke as invoke } from "../../utils/network";
import { t } from "../../utils/i18n";

type ApplyConfigurationResponse = {
  success: boolean;
  message: string;
};

export class TachyonHardwarePanel extends TachyonConfigDashboard {
  private readonly onLanguageChanged = () => { this.render(); this.bindForm(); };

  connectedCallback(): void {
    window.addEventListener("i18n:language-changed", this.onLanguageChanged);
    this.render();
    this.bindForm();
    this.animateEntrance();
  }

  disconnectedCallback(): void {
    window.removeEventListener("i18n:language-changed", this.onLanguageChanged);
  }

  private render(): void {
    this.renderTemplate(`
      <section class="p-6 space-y-6 text-slate-300">
        <div data-stagger-panel class="border-l-4 border-cyan-500 pl-4">
          <h2 class="text-2xl font-bold text-slate-100">${t("hardware.title")}</h2>
          <p class="text-sm font-mono text-slate-400">${t("hardware.subtitle")}</p>
        </div>

        <form class="space-y-6">
          <div data-stagger-panel class="grid grid-cols-1 gap-6 rounded-lg border border-slate-700 bg-slate-800/40 p-6 md:grid-cols-2">
            <label class="block text-xs uppercase tracking-widest text-cyan-500">${t("hardware.field.accelerator")}
              <select id="accelerator" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm text-slate-200 outline-none transition-colors focus:border-cyan-400">
                <option value="npu">${t("hardware.option.npu")}</option>
                <option value="tpu">${t("hardware.option.tpu")}</option>
                <option value="gpu">${t("hardware.option.gpu")}</option>
              </select>
            </label>
            <label class="flex min-h-20 items-center justify-between rounded border border-slate-800 bg-slate-900/50 px-4 text-xs uppercase tracking-widest text-cyan-500">
              <span>${t("hardware.field.ebpf")}</span>
              <input id="xdp-offload" type="checkbox" checked class="h-5 w-5 accent-cyan-500">
            </label>
          </div>

          <button id="apply-hardware" class="bg-cyan-600 px-8 py-3 font-black uppercase tracking-tight text-slate-950 transition-colors hover:bg-cyan-500">
            ${t("hardware.button")}
          </button>
        </form>

        <div id="feedback-zone" data-stagger-panel class="mt-4 rounded-xl border border-slate-800 bg-slate-900 px-4 py-3 font-mono text-xs text-slate-400">${t("hardware.feedback.empty")}</div>
      </section>
    `);
  }

  private bindForm(): void {
    this.root.querySelector("form")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.applyHardwarePolicy();
    });
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
          xdp_offload:
            (this.root.getElementById("xdp-offload") as HTMLInputElement | null)?.checked ?? true,
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

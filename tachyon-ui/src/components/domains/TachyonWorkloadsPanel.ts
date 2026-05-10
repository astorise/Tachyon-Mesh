import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { resilientInvoke as invoke } from "../../utils/network";
import { t } from "../../utils/i18n";

type ApplyConfigurationResponse = {
  success: boolean;
  message: string;
};

export class TachyonWorkloadsPanel extends TachyonConfigDashboard {
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
        <header data-stagger-panel class="flex flex-col gap-3 border-b border-slate-700 pb-4 md:flex-row md:items-center md:justify-between">
          <div>
            <h2 class="text-2xl font-bold text-slate-100">${t("workloads.title")}</h2>
            <p class="text-sm font-mono text-slate-400">${t("workloads.subtitle")}</p>
          </div>
          <span class="w-fit rounded border border-cyan-500/30 bg-cyan-900/50 px-2 py-1 text-[10px] text-cyan-400">${t("workloads.badge")}</span>
        </header>

        <form class="grid grid-cols-1 gap-6 rounded-lg border border-slate-700 bg-slate-800/40 p-6 md:grid-cols-2">
          <label data-stagger-panel class="block text-xs uppercase tracking-widest text-slate-400">${t("workloads.field.engine")}
            <select id="engine" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm text-slate-200 outline-none transition-colors focus:border-cyan-400">
              <option value="wasmtime">${t("workloads.option.wasmtime")}</option>
              <option value="smolvm">${t("workloads.option.smolvm")}</option>
              <option value="legacy">${t("workloads.option.container")}</option>
            </select>
          </label>

          <label data-stagger-panel class="block text-xs uppercase tracking-widest text-slate-400">${t("workloads.field.secret")}
            <input id="secret-ref" type="text" placeholder="${t("workloads.placeholder.secret")}" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm text-slate-200 outline-none transition-colors focus:border-cyan-400">
          </label>

          <button data-stagger-panel class="border border-cyan-500 px-6 py-3 font-bold text-cyan-500 transition-colors hover:bg-cyan-500 hover:text-slate-950 md:col-span-2 md:w-fit">
            ${t("workloads.button")}
          </button>
        </form>

        <div id="feedback-zone" data-stagger-panel class="rounded-xl border border-slate-800 bg-slate-900 px-4 py-3 font-mono text-xs text-slate-400">${t("workloads.feedback.empty")}</div>
      </section>
    `);
  }

  private bindForm(): void {
    this.root.querySelector("form")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.applyWorkloadContract();
    });
  }

  private async applyWorkloadContract(): Promise<void> {
    try {
      const response = await invoke<ApplyConfigurationResponse>("apply_configuration", {
        domain: "workloads",
        payload: {
          engine: this.value("engine", "wasmtime"),
          secret_ref: this.value("secret-ref", ""),
        },
      });
      this.showFeedback(response.success ? "success" : "error", response.message);
    } catch (error) {
      this.showFeedback("error", error instanceof Error ? error.message : String(error));
    }
  }

  private value(id: string, fallback: string): string {
    const value = (
      this.root.getElementById(id) as HTMLInputElement | HTMLSelectElement | null
    )?.value.trim();
    return value ? value : fallback;
  }
}

customElements.define("tachyon-workloads-panel", TachyonWorkloadsPanel);

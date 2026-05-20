import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { applyAndSeal } from "../../utils/network";
import { t } from "../../utils/i18n";
import { loadPolicyConfig, configSourceBadge } from "../../utils/policy-config";
import "../base/TachyonPolicyFormBadge";

export class TachyonFleetPanel extends TachyonConfigDashboard {
  private readonly onLanguageChanged = () => { this.render(); this.bindForm(); };

  async connectedCallback(): Promise<void> {
    window.addEventListener("i18n:language-changed", this.onLanguageChanged);
    this.render();
    this.bindForm();
    this.animateEntrance();
    const config = await loadPolicyConfig("fleet");
    if (config) {
      this.root.querySelector(".flex.items-baseline.gap-2")?.insertAdjacentHTML("beforeend", configSourceBadge(config.source));
      const sel = this.root.getElementById("selector-tags") as HTMLInputElement | null;
      if (sel && typeof config.payload["selector_tags"] === "string") sel.value = config.payload["selector_tags"];
      const profile = this.root.getElementById("node-profile") as HTMLSelectElement | null;
      if (profile && typeof config.payload["node_profile"] === "string") profile.value = config.payload["node_profile"];
    }
  }

  disconnectedCallback(): void {
    window.removeEventListener("i18n:language-changed", this.onLanguageChanged);
  }

  private render(): void {
    this.renderTemplate(`
      <section class="p-6 space-y-6 text-slate-300">
        <header data-stagger-panel class="border-l-4 border-cyan-500 pl-4">
          <div class="flex items-baseline gap-2">
            <h2 class="text-2xl font-bold text-slate-100">${t("fleet.title")}</h2>
            <tachyon-policy-form-badge></tachyon-policy-form-badge>
          </div>
          <p class="text-sm font-mono text-slate-400">${t("fleet.subtitle")}</p>
        </header>

        <form class="grid grid-cols-1 gap-6 rounded-lg border border-slate-700 bg-slate-800/40 p-6 md:grid-cols-2">
          <label data-stagger-panel class="block text-xs uppercase tracking-widest text-cyan-500">${t("fleet.field.selector")}
            <input id="selector-tags" type="text" placeholder="${t("fleet.placeholder.selector")}" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm text-slate-200 outline-none transition-colors focus:border-cyan-400">
          </label>

          <label data-stagger-panel class="block text-xs uppercase tracking-widest text-cyan-500">${t("fleet.field.profile")}
            <select id="node-profile" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm text-slate-200 outline-none transition-colors focus:border-cyan-400">
              <option value="edge-light">${t("fleet.option.edge-light")}</option>
              <option value="core-heavy">${t("fleet.option.core-heavy")}</option>
            </select>
          </label>

          <button data-stagger-panel class="border border-cyan-500 px-6 py-3 font-bold text-cyan-500 transition-colors hover:bg-cyan-500 hover:text-slate-950 md:col-span-2 md:w-fit">
            ${t("fleet.button")}
          </button>
        </form>

        <div id="feedback-zone" data-stagger-panel class="rounded-xl border border-slate-800 bg-slate-900 px-4 py-3 font-mono text-xs text-slate-400">${t("fleet.feedback.empty")}</div>
      </section>
    `);
  }

  private bindForm(): void {
    this.root.querySelector("form")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.applyFleetConfig();
    });
  }

  private async applyFleetConfig(): Promise<void> {
    try {
      const response = await applyAndSeal("fleet", {
          selector_tags: this.value("selector-tags", ""),
          node_profile: this.value("node-profile", "edge-light"),
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

customElements.define("tachyon-fleet-panel", TachyonFleetPanel);

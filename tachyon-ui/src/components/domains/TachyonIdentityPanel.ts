import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { applyAndSeal } from "../../utils/network";
import { t } from "../../utils/i18n";
import "../base/TachyonPolicyFormBadge";

export class TachyonIdentityPanel extends TachyonConfigDashboard {
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
        <header data-stagger-panel class="border-l-4 border-cyan-500 pl-4">
          <div class="flex items-baseline gap-2">
            <h2 class="text-2xl font-bold text-slate-100">${t("identity.title")}</h2>
            <tachyon-policy-form-badge></tachyon-policy-form-badge>
          </div>
          <p class="text-sm font-mono text-slate-400">${t("identity.subtitle")}</p>
        </header>

        <form class="space-y-6 rounded-lg border border-slate-700 bg-slate-800/40 p-6">
          <label data-stagger-panel class="block text-xs uppercase tracking-widest text-cyan-500">${t("identity.field.jwt-issuer")}
            <input id="jwt-issuer" type="url" placeholder="${t("identity.placeholder.jwt")}" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm text-slate-200 outline-none transition-colors focus:border-cyan-400">
          </label>

          <label data-stagger-panel class="block text-xs uppercase tracking-widest text-cyan-500">${t("identity.field.crdt-quota")}
            <input id="crdt-quota" type="number" min="1" step="1" value="10000" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm text-slate-200 outline-none transition-colors focus:border-cyan-400">
          </label>

          <button data-stagger-panel class="border border-cyan-500 px-6 py-3 font-bold text-cyan-500 transition-colors hover:bg-cyan-500 hover:text-slate-950">
            ${t("identity.button")}
          </button>
        </form>

        <div id="feedback-zone" data-stagger-panel class="rounded-xl border border-slate-800 bg-slate-900 px-4 py-3 font-mono text-xs text-slate-400">${t("identity.feedback.empty")}</div>
      </section>
    `);
  }

  private bindForm(): void {
    this.root.querySelector("form")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.applyIdentityConfiguration();
    });
  }

  private async applyIdentityConfiguration(): Promise<void> {
    try {
      const response = await applyAndSeal("config-security", {
          jwt_issuer: this.value("jwt-issuer"),
          crdt_quota: this.numberValue("crdt-quota", 10000),
      });
      this.showFeedback(response.success ? "success" : "error", response.message);
    } catch (error) {
      this.showFeedback("error", error instanceof Error ? error.message : String(error));
    }
  }

  private value(id: string): string {
    return (this.root.getElementById(id) as HTMLInputElement | null)?.value.trim() ?? "";
  }

  private numberValue(id: string, fallback: number): number {
    const input = this.root.getElementById(id) as HTMLInputElement | null;
    const value = input?.valueAsNumber ?? Number.NaN;
    return Number.isFinite(value) ? value : fallback;
  }
}

customElements.define("tachyon-identity-panel", TachyonIdentityPanel);

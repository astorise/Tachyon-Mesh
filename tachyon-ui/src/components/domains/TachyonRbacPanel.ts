import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { applyAndSeal, resilientInvoke as invoke } from "../../utils/network";
import { t } from "../../utils/i18n";
import "../base/TachyonPolicyFormBadge";

export class TachyonRbacPanel extends TachyonConfigDashboard {
  private readonly onLanguageChanged = () => { this.render(); this.bindForm(); };

  async connectedCallback(): Promise<void> {
    window.addEventListener("i18n:language-changed", this.onLanguageChanged);
    this.render();
    this.bindForm();
    this.animateEntrance();
    try {
      const staged = await invoke<Record<string, unknown> | null>("get_staged_config", { domain: "config-rbac" });
      if (staged) {
        const role = this.root.getElementById("role") as HTMLSelectElement | null;
        if (role && typeof staged["role"] === "string") role.value = staged["role"];
        const policy = this.root.getElementById("policy") as HTMLTextAreaElement | null;
        if (policy && staged["policy"] !== undefined)
          policy.value = JSON.stringify(staged["policy"], null, 2);
      }
    } catch { /* staged config unavailable */ }
  }

  disconnectedCallback(): void {
    window.removeEventListener("i18n:language-changed", this.onLanguageChanged);
  }

  private render(): void {
    this.renderTemplate(`
      <section class="p-6 space-y-6 text-slate-300">
        <header data-stagger-panel class="border-l-4 border-cyan-500 pl-4">
          <div class="flex items-baseline gap-2">
            <h2 class="text-2xl font-bold text-slate-100">${t("rbac.title")}</h2>
            <tachyon-policy-form-badge></tachyon-policy-form-badge>
          </div>
          <p class="text-sm font-mono text-slate-400">${t("rbac.subtitle")}</p>
        </header>

        <form class="space-y-6 rounded-lg border border-slate-700 bg-slate-800/40 p-6">
          <label data-stagger-panel class="block text-xs uppercase tracking-widest text-cyan-500">${t("rbac.field.role")}
            <select id="role" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm text-slate-200 outline-none transition-colors focus:border-cyan-400">
              <option value="admin">${t("rbac.option.admin")}</option>
              <option value="ops">${t("rbac.option.ops")}</option>
              <option value="viewer">${t("rbac.option.viewer")}</option>
              <option value="service-account">${t("rbac.option.service-account")}</option>
            </select>
          </label>

          <label data-stagger-panel class="block text-xs uppercase tracking-widest text-cyan-500">${t("rbac.field.policy")}
            <textarea id="policy" rows="12" spellcheck="false" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-3 font-mono text-xs text-slate-200 outline-none transition-colors focus:border-cyan-400">{
  "permissions": [
    {
      "domains": ["config-routing", "config-security"],
      "actions": ["read", "update"]
    }
  ]
}</textarea>
          </label>

          <button data-stagger-panel class="border border-cyan-500 px-6 py-3 font-bold text-cyan-500 transition-colors hover:bg-cyan-500 hover:text-slate-950">
            ${t("rbac.button")}
          </button>
        </form>

        <div id="feedback-zone" data-stagger-panel class="rounded-xl border border-slate-800 bg-slate-900 px-4 py-3 font-mono text-xs text-slate-400">${t("rbac.feedback.empty")}</div>
      </section>
    `);
  }

  private bindForm(): void {
    this.root.querySelector("form")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.applyRbacPolicy();
    });
  }

  private async applyRbacPolicy(): Promise<void> {
    const policyText =
      (this.root.getElementById("policy") as HTMLTextAreaElement | null)?.value.trim() ?? "";
    let policy: unknown;
    try {
      policy = JSON.parse(policyText);
    } catch (error) {
      this.showFeedback(
        "error",
        `${t("rbac.error.invalid-json")} ${error instanceof Error ? error.message : String(error)}`,
      );
      return;
    }

    try {
      const response = await applyAndSeal("config-rbac", {
          role: this.value("role", "admin"),
          policy,
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

customElements.define("tachyon-rbac-panel", TachyonRbacPanel);

import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { resilientInvoke as invoke } from "../../utils/network";

type ApplyConfigurationResponse = {
  success: boolean;
  message: string;
};

export class TachyonRbacPanel extends TachyonConfigDashboard {
  connectedCallback(): void {
    this.render();
    this.root.querySelector("form")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.applyRbacPolicy();
    });
    this.animateEntrance();
  }

  private render(): void {
    this.renderTemplate(`
      <section class="p-6 space-y-6 text-slate-300">
        <header data-stagger-panel class="border-l-4 border-cyan-500 pl-4">
          <h2 class="text-2xl font-bold text-slate-100">RBAC Control Plane</h2>
          <p class="text-sm font-mono text-slate-400">Domain: config-rbac / access policy</p>
        </header>

        <form class="space-y-6 rounded-lg border border-slate-700 bg-slate-800/40 p-6">
          <label data-stagger-panel class="block text-xs uppercase tracking-widest text-cyan-500">Role
            <select id="role" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm text-slate-200 outline-none transition-colors focus:border-cyan-400">
              <option value="admin">Admin</option>
              <option value="ops">Ops</option>
              <option value="viewer">Viewer</option>
              <option value="service-account">Service Account</option>
            </select>
          </label>

          <label data-stagger-panel class="block text-xs uppercase tracking-widest text-cyan-500">Policy (JSON)
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
            Apply RBAC Policy
          </button>
        </form>

        <div id="feedback-zone" data-stagger-panel class="rounded-xl border border-slate-800 bg-slate-900 px-4 py-3 font-mono text-xs text-slate-400">Awaiting RBAC policy.</div>
      </section>
    `);
  }

  private async applyRbacPolicy(): Promise<void> {
    const policyText = (this.root.getElementById("policy") as HTMLTextAreaElement | null)?.value.trim() ?? "";
    let policy: unknown;
    try {
      policy = JSON.parse(policyText);
    } catch (error) {
      this.showFeedback("error", `Policy JSON is invalid: ${error instanceof Error ? error.message : String(error)}`);
      return;
    }

    try {
      const response = await invoke<ApplyConfigurationResponse>("apply_configuration", {
        domain: "config-rbac",
        payload: {
          role: this.value("role", "admin"),
          policy,
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

customElements.define("tachyon-rbac-panel", TachyonRbacPanel);

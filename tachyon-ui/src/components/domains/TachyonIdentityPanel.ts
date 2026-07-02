import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { t } from "../../utils/i18n";
import { getTrustedSigners, writeTrustedSigners } from "../../controllers/manifestConfigController";

export class TachyonIdentityPanel extends TachyonConfigDashboard {
  private readonly onLanguageChanged = () => { this.render(); this.bindForm(); };
  private trustedSigners: string[] = [];

  async connectedCallback(): Promise<void> {
    window.addEventListener("i18n:language-changed", this.onLanguageChanged);
    this.render();
    this.bindForm();
    this.animateEntrance();
    try {
      this.trustedSigners = await getTrustedSigners();
      this.render();
      this.bindForm();
    } catch {
      this.trustedSigners = [];
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
            <h2 class="text-2xl font-bold text-slate-100">${t("identity.title")}</h2>
          </div>
          <p class="text-sm font-mono text-slate-400">${t("identity.subtitle")}</p>
        </header>

        <form id="trusted-signers-form" class="space-y-4 rounded-lg border border-slate-700 bg-slate-800/40 p-6">
          <h3 class="text-sm font-semibold uppercase tracking-widest text-cyan-300">Trusted manifest signers</h3>
          <label class="block text-xs uppercase tracking-widest text-cyan-500">Ed25519 public keys
            <textarea id="trusted-signers" rows="5" placeholder="one 64-character hex key per line" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 font-mono text-xs text-slate-200 outline-none transition-colors focus:border-cyan-400">${this.escape(this.trustedSigners.join("\n"))}</textarea>
          </label>
          <button type="submit" class="rounded border border-cyan-500 px-6 py-3 font-bold text-cyan-500 transition-colors hover:bg-cyan-500 hover:text-slate-950">
            Save trusted signers
          </button>
        </form>

        <div id="feedback-zone" data-stagger-panel class="rounded-xl border border-slate-800 bg-slate-900 px-4 py-3 font-mono text-xs text-slate-400">${t("identity.feedback.empty")}</div>
      </section>
    `);
  }

  private bindForm(): void {
    this.root.querySelector("#trusted-signers-form")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.applyTrustedSigners();
    });
  }

  private async applyTrustedSigners(): Promise<void> {
    try {
      const signers = ((this.root.getElementById("trusted-signers") as HTMLTextAreaElement | null)?.value ?? "")
        .split(/\n|,/);
      const response = await writeTrustedSigners(signers);
      this.trustedSigners = await getTrustedSigners();
      this.render();
      this.bindForm();
      this.showFeedback(response.success ? "success" : "error", response.message);
    } catch (error) {
      this.showFeedback("error", error instanceof Error ? error.message : String(error));
    }
  }

  private escape(value: string): string {
    return value.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
  }
}

customElements.define("tachyon-identity-panel", TachyonIdentityPanel);

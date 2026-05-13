import { tachyonSharedStylesheet } from "../../styles/shared-sheets";
import { t } from "../../utils/i18n";
import { resilientInvoke as invoke } from "../../utils/network";

export type CredentialsSubmittedDetail = {
  url: string;
  username: string;
  password: string;
  cert: number[] | null;
};

/**
 * Presentation component for the initial login form (cluster URL + credentials).
 *
 * Emits `credentials:submitted` when the operator submits the form.
 * Does NOT make any network calls itself — the parent orchestrator (`TachyonIAM`)
 * listens to the event and drives the authentication flow.
 */
export class TachyonAuthStepCredentials extends HTMLElement {
  private readonly root: ShadowRoot;
  private savedUrl = "";
  private readonly onLanguageChanged = () => this.render();

  constructor() {
    super();
    this.root = this.attachShadow({ mode: "open" });
    this.root.adoptedStyleSheets = [tachyonSharedStylesheet];
  }

  connectedCallback(): void {
    window.addEventListener("i18n:language-changed", this.onLanguageChanged);
    this.render();
    this.bindEvents();
    void this.restoreCustomCa();
    void this.restoreCredentials();
  }

  disconnectedCallback(): void {
    window.removeEventListener("i18n:language-changed", this.onLanguageChanged);
  }

  /** Pre-populate the URL field. */
  setUrl(url: string): void {
    this.savedUrl = url;
    const input = this.root.getElementById("cred-url") as HTMLInputElement | null;
    if (input) input.value = url;
  }

  /** Persist credentials to Stronghold if the remember checkbox is checked. */
  async persistIfRemember(url: string, username: string, password: string, cert: number[] | null): Promise<void> {
    const remember = this.root.getElementById("cred-remember") as HTMLInputElement | null;
    if (!remember?.checked) {
      await invoke("delete_credentials").catch(() => undefined);
      return;
    }
    const strongholdAvailable = await invoke<boolean>("stronghold_available").catch(() => false);
    if (!strongholdAvailable) {
      remember.checked = false;
      window.dispatchEvent(new CustomEvent("toast", { detail: { type: "error", message: "Stronghold unavailable — credentials not saved." } }));
      await invoke("delete_credentials").catch(() => undefined);
      return;
    }
    await invoke("save_credentials", { payload: { url, username, password, customCa: cert } });
  }

  private render(): void {
    this.root.innerHTML = `
      <form id="cred-form" class="space-y-3" aria-label="${t("iam.login.form-label")}">
        <label for="cred-url" class="sr-only">${t("iam.placeholder.url")}</label>
        <input type="text" id="cred-url" value="${this.savedUrl}" placeholder="${t("iam.placeholder.url")}" aria-required="true" autocomplete="url"
          class="w-full bg-slate-950 border border-slate-700 p-3 rounded-lg text-white text-sm font-mono" />

        <label for="cred-username" class="sr-only">${t("iam.placeholder.username")}</label>
        <input type="text" id="cred-username" placeholder="${t("iam.placeholder.username")}" aria-required="true" autocomplete="username"
          class="w-full bg-slate-950 border border-slate-700 p-3 rounded-lg text-white text-sm" />

        <div class="flex gap-2">
          <label for="cred-password" class="sr-only">${t("iam.placeholder.password")}</label>
          <input type="password" id="cred-password" placeholder="${t("iam.placeholder.password")}" aria-required="true" autocomplete="current-password"
            class="min-w-0 flex-1 bg-slate-950 border border-slate-700 p-3 rounded-lg text-white text-sm" />
          <button type="button" id="cred-toggle-pw" class="rounded-lg border border-slate-700 bg-slate-800 px-3 text-sm text-slate-300">
            ${t("iam.password.show")}
          </button>
        </div>

        <label class="flex items-center gap-3 rounded-lg border border-slate-800 bg-slate-950/70 px-3 py-2 text-xs text-slate-300">
          <input type="checkbox" id="cred-remember" class="h-4 w-4 accent-cyan-500" />
          <span>${t("iam.remember")}</span>
        </label>

        <input type="file" id="cred-mtls" accept=".pem,.crt,.cer"
          class="w-full text-slate-400 text-sm file:mr-4 file:py-2 file:px-4 file:rounded-lg file:border-0 file:bg-cyan-500/10 file:text-cyan-400 cursor-pointer hover:file:bg-cyan-500/20" />
        <div id="cred-ca-status" class="rounded-lg border border-slate-800 bg-slate-950/70 px-3 py-2 text-xs text-slate-400">${t("iam.ca.none")}</div>

        <div class="grid gap-2 md:grid-cols-2">
          <button type="button" id="cred-save-ca" class="rounded-lg border border-slate-700 bg-slate-800 px-3 py-2 text-xs text-slate-300 hover:bg-slate-700">${t("iam.ca.save")}</button>
          <button type="button" id="cred-clear-ca" class="rounded-lg border border-slate-700 bg-slate-800 px-3 py-2 text-xs text-slate-300 hover:bg-slate-700">${t("iam.ca.clear")}</button>
        </div>

        <button type="submit" id="cred-submit" class="w-full bg-cyan-600 hover:bg-cyan-500 py-3 rounded-xl text-white font-bold transition-all shadow-[0_0_15px_rgba(34,211,238,0.2)]">
          ${t("iam.authenticate")}
        </button>
      </form>
    `;
  }

  private bindEvents(): void {
    this.root.getElementById("cred-form")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.submit();
    });
    this.root.getElementById("cred-toggle-pw")?.addEventListener("click", () => {
      const pw = this.root.getElementById("cred-password") as HTMLInputElement | null;
      const btn = this.root.getElementById("cred-toggle-pw") as HTMLButtonElement | null;
      if (!pw) return;
      pw.type = pw.type === "password" ? "text" : "password";
      if (btn) btn.textContent = pw.type === "password" ? t("iam.password.show") : t("iam.password.hide");
    });
    this.root.getElementById("cred-save-ca")?.addEventListener("click", () => void this.saveSelectedCa());
    this.root.getElementById("cred-clear-ca")?.addEventListener("click", () => void this.clearCa());
    this.root.getElementById("cred-url")?.addEventListener("input", (event) => {
      this.savedUrl = (event.target as HTMLInputElement).value.trim();
      this.dispatchEvent(new CustomEvent("credentials:url-changed", {
        bubbles: true,
        composed: true,
        detail: { url: this.savedUrl },
      }));
    });
  }

  private async submit(): Promise<void> {
    const url = this.value("cred-url");
    const username = this.value("cred-username");
    const password = this.value("cred-password");
    if (!url || !username || !password) return;
    const cert = await this.currentCert();
    this.dispatchEvent(
      new CustomEvent<CredentialsSubmittedDetail>("credentials:submitted", {
        bubbles: true,
        composed: true,
        detail: { url, username, password, cert },
      }),
    );
  }

  private async saveSelectedCa(): Promise<void> {
    const cert = await this.readSelectedCert();
    if (!cert) {
      this.notifyCaStatus(0);
      return;
    }
    await invoke("save_custom_ca", { cert });
    this.notifyCaStatus(cert.length);
  }

  private async clearCa(): Promise<void> {
    await invoke("clear_custom_ca");
    this.notifyCaStatus(0);
  }

  private async restoreCustomCa(): Promise<void> {
    try {
      const cert = await invoke<number[] | null>("load_custom_ca");
      this.notifyCaStatus(cert?.length ?? 0);
    } catch {
      this.notifyCaStatus(0);
    }
  }

  private notifyCaStatus(byteLength: number): void {
    const status = this.root.getElementById("cred-ca-status");
    if (!status) return;
    status.textContent = byteLength > 0
      ? `${t("iam.ca.loaded.prefix")} (${byteLength} ${t("iam.ca.bytes")}).`
      : t("iam.ca.none");
  }

  private async currentCert(): Promise<number[] | null> {
    const selected = await this.readSelectedCert();
    if (selected) return selected;
    return invoke<number[] | null>("load_custom_ca").catch(() => null);
  }

  private async restoreCredentials(): Promise<void> {
    try {
      const saved = await invoke<Partial<{ url: string; username: string; password: string }> | null>("load_credentials");
      if (!saved) return;
      if (saved.url) this.setUrl(saved.url);
      if (saved.username) {
        const u = this.root.getElementById("cred-username") as HTMLInputElement | null;
        if (u) u.value = saved.username;
      }
      if (saved.password) {
        const p = this.root.getElementById("cred-password") as HTMLInputElement | null;
        if (p) p.value = saved.password;
      }
      const remember = this.root.getElementById("cred-remember") as HTMLInputElement | null;
      if (remember) remember.checked = true;
    } catch {
      await invoke("delete_credentials").catch(() => undefined);
    }
  }

  private async readSelectedCert(): Promise<number[] | null> {
    const file = (this.root.getElementById("cred-mtls") as HTMLInputElement | null)?.files?.[0];
    if (!file) return null;
    return Array.from(new Uint8Array(await file.arrayBuffer()));
  }

  private value(id: string): string {
    return (this.root.getElementById(id) as HTMLInputElement | null)?.value.trim() ?? "";
  }
}

customElements.define("auth-step-credentials", TachyonAuthStepCredentials);

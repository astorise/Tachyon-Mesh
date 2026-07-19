import stylesheetText from "../../style.css?inline";
import { loggedInvoke as tauriInvoke } from "../../utils/appLogger";
import { t } from "../../utils/i18n";

const mfaStylesheet = new CSSStyleSheet();
mfaStylesheet.replaceSync(stylesheetText);

export class TachyonMfaPrompt extends HTMLElement {
  private readonly root: ShadowRoot;
  private resolver: (() => void) | null = null;
  private rejecter: ((error: Error) => void) | null = null;

  constructor() {
    super();
    this.root = this.attachShadow({ mode: "open" });
    this.root.adoptedStyleSheets = [mfaStylesheet];
  }

  connectedCallback(): void {
    this.render();
    this.bindEvents();
  }

  open(): Promise<void> {
    this.dialog()?.showModal();
    this.input()?.focus();
    return new Promise((resolve, reject) => {
      this.resolver = resolve;
      this.rejecter = reject;
    });
  }

  private render(): void {
    this.root.innerHTML = `
      <dialog id="mfa-dialog" role="dialog" aria-modal="true" aria-labelledby="mfa-title" aria-describedby="mfa-desc" class="bg-transparent p-0 backdrop:bg-slate-950/80 backdrop:backdrop-blur-sm open:flex fixed inset-0 z-[100] h-screen w-screen items-center justify-center">
        <form id="mfa-form" class="w-full max-w-sm rounded-xl border border-slate-800 bg-slate-900 p-6 shadow-2xl">
          <div class="mb-2 flex items-center gap-3 text-amber-500">
            <span class="inline-flex h-6 w-6 items-center justify-center rounded border border-amber-500/50 text-sm" aria-hidden="true">!</span>
            <h3 id="mfa-title" class="text-lg font-medium">${t("mfa.title")}</h3>
          </div>
          <p id="mfa-desc" class="mb-4 text-sm text-slate-400">${t("mfa.body")}</p>
          <label for="mfa-input" class="sr-only">${t("mfa.code-label")}</label>
          <input type="text" id="mfa-input" placeholder="000000" maxlength="6" pattern="[0-9]{6}" aria-required="true" aria-label="${t("mfa.code-label")}" aria-describedby="mfa-error" autocomplete="one-time-code" inputmode="numeric" class="mb-6 w-full rounded-md border border-slate-700 bg-slate-950 px-4 py-3 text-center text-2xl font-mono tracking-[0.5em] text-cyan-300 focus:border-cyan-500 focus:outline-none" required>
          <div id="mfa-error" role="alert" class="mb-4 hidden rounded border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300"></div>
          <div class="flex justify-end gap-3">
            <button type="button" id="mfa-cancel" class="px-4 py-2 text-sm text-slate-400 hover:text-white">${t("mfa.cancel")}</button>
            <button type="submit" class="rounded-md border border-cyan-500/50 bg-cyan-500/20 px-4 py-2 text-sm text-cyan-300">${t("mfa.verify")}</button>
          </div>
        </form>
      </dialog>
    `;
  }

  private bindEvents(): void {
    this.root.getElementById("mfa-cancel")?.addEventListener("click", () => this.cancel());
    this.form()?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.verify();
    });
  }

  private async verify(): Promise<void> {
    const code = this.input()?.value.trim() ?? "";
    if (!/^\d{6}$/.test(code)) {
      this.showError(t("mfa.format-error"));
      return;
    }
    try {
      await tauriInvoke("verify_session_totp", { code });
      this.dialog()?.close();
      this.input()!.value = "";
      this.resolver?.();
      this.resolver = null;
      this.rejecter = null;
    } catch (error) {
      this.showError(error instanceof Error ? error.message : String(error));
    }
  }

  private cancel(): void {
    this.dialog()?.close();
    this.input()!.value = "";
    this.rejecter?.(new Error(t("mfa.cancelled")));
    this.resolver = null;
    this.rejecter = null;
  }

  private showError(message: string): void {
    const error = this.root.getElementById("mfa-error");
    if (!error) {
      return;
    }
    error.textContent = message;
    error.classList.remove("hidden");
  }

  private dialog(): HTMLDialogElement | null {
    return this.root.getElementById("mfa-dialog") as HTMLDialogElement | null;
  }

  private form(): HTMLFormElement | null {
    return this.root.getElementById("mfa-form") as HTMLFormElement | null;
  }

  private input(): HTMLInputElement | null {
    return this.root.getElementById("mfa-input") as HTMLInputElement | null;
  }
}

customElements.define("tachyon-mfa-prompt", TachyonMfaPrompt);

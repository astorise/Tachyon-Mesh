import gsap from "gsap";
import QRCode from "qrcode";

import stylesheetText from "../../style.css?inline";
import { t } from "../../utils/i18n";
import { resilientInvoke as invoke } from "../../utils/network";

type AuthLoginResponse = {
  username: string;
  endpoint: string;
  requiresMfa: boolean;
  sessionId?: string | null;
};

type RegistrationTokenClaims = {
  subject: string;
  roles: string[];
  scopes: string[];
  expiresAt: number;
};

type StagedSignupSession = {
  sessionId: string;
  username: string;
  provisioningUri: string;
  roles: string[];
  scopes: string[];
  expiresAt: number;
};

type AuthenticatedDetail = {
  user: string;
  role: string;
  token: string;
};

type EnrollmentInvite = {
  sessionId: string;
  inviteToken: string;
  qrPayload: string;
};

const iamStylesheet = new CSSStyleSheet();
iamStylesheet.replaceSync(stylesheetText);

export class TachyonIAM extends HTMLElement {
  private readonly root: ShadowRoot;
  private activeStep: "login" | "mfa" | "signup-token" | "signup-profile" | "signup-totp" = "login";
  private claims: RegistrationTokenClaims | null = null;
  private stagedSignup: StagedSignupSession | null = null;
  private stagedLogin: AuthLoginResponse | null = null;

  constructor() {
    super();
    this.root = this.attachShadow({ mode: "open" });
    this.root.adoptedStyleSheets = [iamStylesheet];
  }

  connectedCallback(): void {
    this.render();
    this.bindEvents();
    if (this.mode() === "auth") {
      void gsap.fromTo(this.panel(), { y: 16, opacity: 0 }, { y: 0, opacity: 1, duration: 0.35 });
    }
  }

  private render(): void {
    if (this.mode() === "admin") {
      this.renderAdminPanel();
      return;
    }
    this.root.innerHTML = `
      <section class="fixed inset-0 z-[100] bg-slate-950/95 backdrop-blur-xl flex items-center justify-center text-slate-300">
        <div id="iam-panel" class="bg-slate-900 border border-slate-800 p-8 rounded-2xl w-full max-w-2xl shadow-2xl relative overflow-hidden">
          <div class="absolute top-0 left-0 w-full h-1 bg-gradient-to-r from-cyan-600 to-blue-500"></div>
          <h2 class="text-white text-2xl font-bold mb-1">${t("iam.title")}</h2>
          <p class="text-slate-400 text-sm mb-6">${t("iam.subtitle")}</p>
          <form id="auth-step-login" class="auth-step space-y-3">
            <input type="text" id="auth-url" placeholder="${t("iam.placeholder.url")}" class="w-full bg-slate-950 border border-slate-700 p-3 rounded-lg text-white text-sm font-mono" />
            <input type="text" id="auth-username" placeholder="${t("iam.placeholder.username")}" class="w-full bg-slate-950 border border-slate-700 p-3 rounded-lg text-white text-sm" />
            <div class="flex gap-2">
              <input type="password" id="auth-password" placeholder="${t("iam.placeholder.password")}" class="min-w-0 flex-1 bg-slate-950 border border-slate-700 p-3 rounded-lg text-white text-sm" />
              <button type="button" id="btn-toggle-login-password" class="rounded-lg border border-slate-700 bg-slate-800 px-3 text-sm text-slate-300">${t("iam.password.show")}</button>
            </div>
            <label class="flex items-center gap-3 rounded-lg border border-slate-800 bg-slate-950/70 px-3 py-2 text-xs text-slate-300">
              <input type="checkbox" id="remember-credentials" class="h-4 w-4 accent-cyan-500" />
              <span>${t("iam.remember")}</span>
            </label>
            <input type="file" id="auth-mtls" class="w-full text-slate-400 text-sm file:mr-4 file:py-2 file:px-4 file:rounded-lg file:border-0 file:bg-cyan-500/10 file:text-cyan-400 cursor-pointer hover:file:bg-cyan-500/20" />
            <div id="auth-ca-status" class="rounded-lg border border-slate-800 bg-slate-950/70 px-3 py-2 text-xs text-slate-400">${t("iam.ca.none")}</div>
            <div class="grid gap-2 md:grid-cols-2">
              <button type="button" id="btn-save-custom-ca" class="rounded-lg border border-slate-700 bg-slate-800 px-3 py-2 text-xs text-slate-300 hover:bg-slate-700">${t("iam.ca.save")}</button>
              <button type="button" id="btn-clear-custom-ca" class="rounded-lg border border-slate-700 bg-slate-800 px-3 py-2 text-xs text-slate-300 hover:bg-slate-700">${t("iam.ca.clear")}</button>
            </div>
            <div class="grid gap-3 md:grid-cols-2 pt-3">
              <button id="btn-login-submit" class="w-full bg-cyan-600 hover:bg-cyan-500 py-3 rounded-xl text-white font-bold transition-all shadow-[0_0_15px_rgba(34,211,238,0.2)]">${t("iam.authenticate")}</button>
              <button type="button" id="btn-open-signup" class="w-full bg-slate-800 hover:bg-slate-700 py-3 rounded-xl text-white font-semibold transition-all border border-slate-700">${t("iam.register")}</button>
            </div>
          </form>
          <form id="auth-step-mfa" class="auth-step hidden space-y-4">
            <div class="rounded-xl border border-cyan-500/20 bg-cyan-500/5 px-4 py-3 text-sm text-cyan-200">${t("iam.mfa.prompt")}</div>
            <input type="text" id="auth-mfa-code" placeholder="000000" maxlength="6" class="w-full bg-slate-950 border border-slate-700 p-4 rounded-lg text-cyan-400 text-center text-3xl tracking-[0.35em] font-mono focus:border-cyan-500 focus:outline-none" />
            <button id="btn-mfa-submit" class="w-full bg-cyan-600 hover:bg-cyan-500 py-3 rounded-xl text-white font-bold transition-all">${t("iam.mfa.verify")}</button>
          </form>
          <form id="auth-step-signup-token" class="auth-step hidden space-y-4">
            <div class="rounded-xl border border-cyan-500/20 bg-cyan-500/5 px-4 py-3 text-sm text-cyan-200">${t("iam.signup.intro")}</div>
            <input type="text" id="signup-auth-url" placeholder="${t("iam.placeholder.url")}" class="w-full bg-slate-950 border border-slate-700 p-3 rounded-lg text-white text-sm font-mono" />
            <input type="password" id="signup-token" placeholder="${t("iam.placeholder.invite")}" class="w-full bg-slate-950 border border-slate-700 p-3 rounded-lg text-white text-sm font-mono" />
            <div class="grid gap-3 md:grid-cols-2">
              <button type="button" id="btn-signup-token-back" class="w-full bg-slate-800 hover:bg-slate-700 py-3 rounded-xl text-white font-semibold transition-all border border-slate-700">${t("iam.signup.back-to-login")}</button>
              <button id="btn-signup-token-submit" class="w-full bg-cyan-600 hover:bg-cyan-500 py-3 rounded-xl text-white font-bold transition-all">${t("iam.signup.validate-invite")}</button>
            </div>
          </form>
          <form id="auth-step-signup-profile" class="auth-step hidden space-y-3">
            <div class="grid gap-3 md:grid-cols-2">
              <input type="text" id="signup-first-name" placeholder="${t("iam.placeholder.first-name")}" class="w-full bg-slate-950 border border-slate-700 p-3 rounded-lg text-white text-sm" />
              <input type="text" id="signup-last-name" placeholder="${t("iam.placeholder.last-name")}" class="w-full bg-slate-950 border border-slate-700 p-3 rounded-lg text-white text-sm" />
            </div>
            <input type="text" id="signup-username" placeholder="${t("iam.placeholder.username-auto")}" readonly class="w-full bg-slate-950 border border-slate-700 p-3 rounded-lg text-slate-300 text-sm font-mono cursor-not-allowed" />
            <input type="password" id="signup-password" placeholder="${t("iam.placeholder.password-min")}" class="w-full bg-slate-950 border border-slate-700 p-3 rounded-lg text-white text-sm" />
            <input type="password" id="signup-confirm-password" placeholder="${t("iam.placeholder.password-confirm")}" class="w-full bg-slate-950 border border-slate-700 p-3 rounded-lg text-white text-sm" />
            <p id="signup-token-summary" class="text-xs text-slate-500">${t("iam.signup.no-invite")}</p>
            <div class="grid gap-3 md:grid-cols-2 pt-2">
              <button type="button" id="btn-signup-profile-back" class="w-full bg-slate-800 hover:bg-slate-700 py-3 rounded-xl text-white font-semibold transition-all border border-slate-700">${t("iam.back")}</button>
              <button id="btn-signup-profile-submit" class="w-full bg-cyan-600 hover:bg-cyan-500 py-3 rounded-xl text-white font-bold transition-all">${t("iam.signup.stage-account")}</button>
            </div>
          </form>
          <form id="auth-step-signup-totp" class="auth-step hidden space-y-4">
            <div class="grid gap-4 lg:grid-cols-[0.9fr_1.1fr]">
              <div class="rounded-2xl border border-slate-800 bg-slate-950/70 p-5">
                <div class="mb-2 text-xs uppercase tracking-[0.2em] text-slate-500">${t("iam.signup.qr-title")}</div>
                <div id="signup-totp-qr" class="flex min-h-64 items-center justify-center rounded-2xl border border-dashed border-slate-700 bg-slate-900 p-4 text-center text-sm text-slate-500">
                  ${t("iam.signup.qr-waiting")}
                </div>
              </div>
              <div class="rounded-2xl border border-slate-800 bg-slate-950/70 p-5 space-y-4">
                <div>
                  <div class="mb-2 text-xs uppercase tracking-[0.2em] text-slate-500">${t("iam.signup.session")}</div>
                  <div id="signup-session-id" class="break-all font-mono text-sm text-cyan-300">${t("iam.signup.pending")}</div>
                </div>
                <div>
                  <div class="mb-2 text-xs uppercase tracking-[0.2em] text-slate-500">${t("iam.signup.manual-secret")}</div>
                  <div id="signup-manual-secret" class="break-all font-mono text-sm text-white">${t("iam.signup.scan-when-ready")}</div>
                </div>
              </div>
            </div>
            <input type="text" id="signup-totp-code" placeholder="000000" maxlength="6" class="w-full bg-slate-950 border border-slate-700 p-4 rounded-lg text-cyan-400 text-center text-3xl tracking-[0.35em] font-mono focus:border-cyan-500 focus:outline-none" />
            <div class="grid gap-3 md:grid-cols-2">
              <button type="button" id="btn-signup-totp-back" class="w-full bg-slate-800 hover:bg-slate-700 py-3 rounded-xl text-white font-semibold transition-all border border-slate-700">${t("iam.back")}</button>
              <button id="btn-signup-totp-submit" class="w-full bg-cyan-600 hover:bg-cyan-500 py-3 rounded-xl text-white font-bold transition-all">${t("iam.signup.finalize")}</button>
            </div>
          </form>
          <div id="auth-error" class="hidden mt-4 rounded-lg border border-red-500/20 bg-red-500/10 px-3 py-2 text-xs text-red-300 text-center">${t("iam.error.generic")}</div>
        </div>
      </section>
    `;
  }

  private bindEvents(): void {
    if (this.mode() === "admin") {
      this.bindAdminEvents();
      return;
    }
    this.form("auth-step-login")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.login();
    });
    this.button("btn-toggle-login-password")?.addEventListener("click", () => this.togglePassword("auth-password"));
    this.input("remember-credentials")?.addEventListener("change", () => void this.persistCredentialsPreference());
    this.button("btn-save-custom-ca")?.addEventListener("click", () => void this.saveSelectedCustomCa());
    this.button("btn-clear-custom-ca")?.addEventListener("click", () => void this.clearCustomCa());
    this.input("auth-url")?.addEventListener("input", () => this.syncAuthUrls("auth-url", "signup-auth-url"));
    this.input("signup-auth-url")?.addEventListener("input", () => this.syncAuthUrls("signup-auth-url", "auth-url"));
    this.form("auth-step-mfa")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.finalizeLogin();
    });
    this.button("btn-open-signup")?.addEventListener("click", () => void this.switchStep("signup-token"));
    this.button("btn-signup-token-back")?.addEventListener("click", () => void this.switchStep("login"));
    this.form("auth-step-signup-token")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.validateInvite();
    });
    this.button("btn-signup-profile-back")?.addEventListener("click", () => void this.switchStep("signup-token"));
    this.form("auth-step-signup-profile")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.stageAccount();
    });
    this.button("btn-signup-totp-back")?.addEventListener("click", () => void this.switchStep("signup-profile"));
    this.form("auth-step-signup-totp")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.finalizeSignup();
    });
    this.input("signup-first-name")?.addEventListener("input", () => this.updateUsername());
    this.input("signup-last-name")?.addEventListener("input", () => this.updateUsername());
    void this.restoreSavedCredentials();
    void this.restoreCustomCa();
  }

  private renderAdminPanel(): void {
    this.root.innerHTML = `
      <section class="rounded-xl border border-emerald-500/30 bg-emerald-500/10 p-6 backdrop-blur-sm">
        <h3 class="mb-2 text-lg font-medium text-emerald-400">${t("iam.admin.title")}</h3>
        <p class="mb-4 text-sm text-slate-400">${t("iam.admin.subtitle")}</p>
        <button id="btn-generate-invite" class="rounded-md border border-emerald-500/50 bg-emerald-500/20 px-4 py-2 text-sm font-medium text-emerald-300 transition-colors hover:bg-emerald-500/30">
          ${t("iam.admin.generate")}
        </button>
        <div id="invite-result-container" class="mt-6 hidden gap-6">
          <div id="invite-qr-code" class="rounded-md bg-white p-2"></div>
          <div class="flex flex-col justify-center">
            <span class="text-xs uppercase text-slate-500">${t("iam.admin.manual-token")}</span>
            <span id="invite-token-display" class="mt-1 font-mono text-2xl tracking-widest text-emerald-300"></span>
            <span id="invite-session-display" class="mt-2 break-all font-mono text-xs text-slate-500"></span>
          </div>
        </div>
      </section>
    `;
  }

  private bindAdminEvents(): void {
    this.button("btn-generate-invite")?.addEventListener("click", () => void this.generateInvite());
  }

  private async generateInvite(): Promise<void> {
    try {
      const invite = await invoke<EnrollmentInvite>("generate_operator_invite");
      const container = this.root.getElementById("invite-result-container");
      const token = this.root.getElementById("invite-token-display");
      const session = this.root.getElementById("invite-session-display");
      const qr = this.root.getElementById("invite-qr-code");
      if (token) {
        token.textContent = invite.inviteToken;
      }
      if (session) {
        session.textContent = invite.sessionId;
      }
      if (qr) {
        qr.innerHTML = await QRCode.toString(invite.qrPayload, {
          type: "svg",
          margin: 1,
          width: 180,
          color: { dark: "#020617", light: "#ffffff" },
        });
      }
      container?.classList.remove("hidden");
      container?.classList.add("flex");
      this.notify("success", t("iam.admin.invite-generated"));
    } catch (error) {
      this.notify("error", error instanceof Error ? error.message : String(error));
    }
  }

  private async login(): Promise<void> {
    const url = this.value("auth-url");
    const username = this.value("auth-username");
    const password = this.value("auth-password");
    if (!url || !username || !password) {
      this.showError(t("iam.error.login-required"));
      return;
    }
    try {
      const response = await invoke<AuthLoginResponse>("authn_login", {
        payload: { url, username, password, cert: await this.currentCustomCa() },
      });
      if (!response.sessionId) {
        throw new Error(t("iam.error.no-session"));
      }
      await this.persistCredentialsPreference();
      this.stagedLogin = response;
      await this.switchStep("mfa");
    } catch (error) {
      this.emitError(error, "authn_login_failed");
    }
  }

  private async finalizeLogin(): Promise<void> {
    if (!this.stagedLogin?.sessionId) {
      this.showError(t("iam.error.no-staged-login"));
      return;
    }
    const totpCode = this.value("auth-mfa-code").replace(/\s+/g, "");
    if (!/^\d{6}$/.test(totpCode)) {
      this.showError(t("iam.error.totp-format"));
      return;
    }
    try {
      const response = await invoke<AuthLoginResponse>("finalize_login", {
        payload: {
          url: this.value("auth-url"),
          sessionId: this.stagedLogin.sessionId,
          totpCode,
          cert: await this.currentCustomCa(),
        },
      });
      this.stagedLogin = null;
      this.clearPasswordFields();
      await this.completeAuthentication({ user: response.username, role: "admin", token: "" });
    } catch (error) {
      this.emitError(error, "authn_mfa_failed");
    }
  }

  private async validateInvite(): Promise<void> {
    if (!this.signupUrl()) {
      this.showError(t("iam.error.url-required"));
      return;
    }
    try {
      this.claims = await invoke<RegistrationTokenClaims>("validate_signup_token", {
        payload: { url: this.signupUrl(), token: this.value("signup-token"), cert: await this.currentCustomCa() },
      });
      const summary = this.root.getElementById("signup-token-summary");
      if (summary) {
        const noneLabel = t("iam.signup.none");
        summary.textContent = `${this.claims.subject} | ${t("iam.signup.roles")}=${this.claims.roles.join(", ") || noneLabel} | ${t("iam.signup.scopes")}=${this.claims.scopes.join(", ") || noneLabel}`;
      }
      await this.switchStep("signup-profile");
    } catch (error) {
      this.emitError(error, "signup_token_failed");
    }
  }

  private async stageAccount(): Promise<void> {
    if (!this.signupUrl()) {
      this.showError(t("iam.error.url-required"));
      return;
    }
    const password = this.value("signup-password");
    if (password.length < 8 || password !== this.value("signup-confirm-password")) {
      this.showError(t("iam.error.password-rules"));
      return;
    }
    try {
      this.stagedSignup = await invoke<StagedSignupSession>("stage_signup", {
        payload: {
          url: this.signupUrl(),
          token: this.value("signup-token"),
          firstName: this.value("signup-first-name"),
          lastName: this.value("signup-last-name"),
          username: this.value("signup-username"),
          password,
          cert: await this.currentCustomCa(),
        },
      });
      await this.renderTotpEnrollment(this.stagedSignup);
      this.clearSignupPasswordFields();
      await this.switchStep("signup-totp");
    } catch (error) {
      this.emitError(error, "signup_stage_failed");
    }
  }

  private async finalizeSignup(): Promise<void> {
    if (!this.stagedSignup) {
      this.showError(t("iam.error.no-staged-signup"));
      return;
    }
    if (!this.signupUrl()) {
      this.showError(t("iam.error.url-required"));
      return;
    }
    try {
      const response = await invoke<AuthLoginResponse>("finalize_signup", {
        payload: {
          url: this.signupUrl(),
          sessionId: this.stagedSignup.sessionId,
          totpCode: this.value("signup-totp-code"),
          cert: await this.currentCustomCa(),
        },
      });
      await this.completeAuthentication({
        user: response.username,
        role: this.claims?.roles[0] ?? "admin",
        token: this.value("signup-token"),
      });
    } catch (error) {
      this.emitError(error, "signup_finalize_failed");
    }
  }

  private async completeAuthentication(detail: AuthenticatedDetail): Promise<void> {
    this.hideError();
    this.dispatchEvent(new CustomEvent("iam:authenticated", { bubbles: true, composed: true, detail }));
    await gsap.to(this.panel(), { y: -12, opacity: 0, duration: 0.2 });
    this.classList.add("hidden");
  }

  private async switchStep(step: typeof this.activeStep): Promise<void> {
    const current = this.root.getElementById(`auth-step-${this.activeStep}`);
    const next = this.root.getElementById(`auth-step-${step}`);
    if (!current || !next || current === next) {
      return;
    }
    this.hideError();
    await gsap.to(current, { opacity: 0, duration: 0.12 });
    current.classList.add("hidden");
    next.classList.remove("hidden");
    gsap.set(next, { opacity: 0 });
    await gsap.to(next, { opacity: 1, duration: 0.16 });
    this.activeStep = step;
  }

  private updateUsername(): void {
    const username = this.input("signup-username");
    if (!username) {
      return;
    }
    const first = this.value("signup-first-name").toLowerCase();
    const last = this.value("signup-last-name").toLowerCase();
    username.value = [first, last].filter(Boolean).join(".").replace(/[^a-z0-9._-]/g, "");
  }

  private async renderTotpEnrollment(session: StagedSignupSession): Promise<void> {
    const qr = this.root.getElementById("signup-totp-qr");
    const sessionId = this.root.getElementById("signup-session-id");
    const manualSecret = this.root.getElementById("signup-manual-secret");

    if (sessionId) {
      sessionId.textContent = session.sessionId;
    }
    if (manualSecret) {
      manualSecret.textContent = this.extractProvisioningSecret(session.provisioningUri);
    }
    if (!qr) {
      return;
    }

    try {
      qr.innerHTML = await QRCode.toString(session.provisioningUri, {
        type: "svg",
        margin: 1,
        width: 256,
        color: {
          dark: "#e2e8f0",
          light: "#0f172a",
        },
      });
      qr.className =
        "flex min-h-64 items-center justify-center rounded-2xl border border-slate-700 bg-slate-900 p-4";
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      qr.textContent = `${t("iam.error.qr")} ${t("iam.signup.manual-secret")}: ${this.extractProvisioningSecret(session.provisioningUri)}`;
      this.dispatchEvent(
        new CustomEvent("iam:error", {
          bubbles: true,
          composed: true,
          detail: { message, code: "signup_qr_failed" },
        }),
      );
    }
  }

  private extractProvisioningSecret(provisioningUri: string): string {
    try {
      return new URL(provisioningUri).searchParams.get("secret") || provisioningUri;
    } catch {
      return provisioningUri;
    }
  }

  private emitError(error: unknown, code: string): void {
    const message = error instanceof Error ? error.message : String(error);
    this.showError(message);
    this.dispatchEvent(new CustomEvent("iam:error", { bubbles: true, composed: true, detail: { message, code } }));
  }

  private togglePassword(id: string): void {
    const input = this.input(id);
    const button = this.button("btn-toggle-login-password");
    if (!input) {
      return;
    }
    input.type = input.type === "password" ? "text" : "password";
    if (button) {
      button.textContent = input.type === "password" ? t("iam.password.show") : t("iam.password.hide");
    }
  }

  private async restoreSavedCredentials(): Promise<void> {
    try {
      const parsed = await invoke<Partial<{ url: string; username: string; password: string; customCa: number[] }> | null>("load_credentials");
      if (!parsed) {
        return;
      }
      if (parsed.url) {
        const url = this.input("auth-url");
        if (url) {
          url.value = parsed.url;
        }
        const signupUrl = this.input("signup-auth-url");
        if (signupUrl) {
          signupUrl.value = parsed.url;
        }
      }
      if (parsed.username) {
        const username = this.input("auth-username");
        if (username) {
          username.value = parsed.username;
        }
      }
      if (parsed.password) {
        const password = this.input("auth-password");
        if (password) {
          password.value = parsed.password;
        }
      }
      const remember = this.input("remember-credentials");
      if (remember) {
        remember.checked = true;
      }
    } catch {
      await invoke("delete_credentials");
    }
  }

  private async restoreCustomCa(): Promise<void> {
    try {
      const cert = await invoke<number[] | null>("load_custom_ca");
      this.updateCaStatus(cert?.length ?? 0);
    } catch {
      this.updateCaStatus(0);
    }
  }

  private async persistCredentialsPreference(): Promise<void> {
    if (!this.input("remember-credentials")?.checked) {
      await invoke("delete_credentials");
      return;
    }
    this.notify("success", t("iam.credentials.stronghold-stored"));
    await invoke("save_credentials", {
      payload: {
        url: this.value("auth-url"),
        username: this.value("auth-username"),
        password: this.input("auth-password")?.value ?? "",
        customCa: await this.currentCustomCa(),
      },
    });
  }

  private async saveSelectedCustomCa(): Promise<void> {
    const cert = await this.readSelectedCert();
    if (!cert) {
      this.notify("error", t("iam.ca.select-first"));
      return;
    }
    await invoke("save_custom_ca", { cert });
    this.updateCaStatus(cert.length);
    this.notify("success", t("iam.ca.saved"));
  }

  private async clearCustomCa(): Promise<void> {
    await invoke("clear_custom_ca");
    this.updateCaStatus(0);
    this.notify("success", t("iam.ca.cleared"));
  }

  private async currentCustomCa(): Promise<number[] | null> {
    const selected = await this.readSelectedCert();
    if (selected) {
      return selected;
    }
    return await invoke<number[] | null>("load_custom_ca");
  }

  private async readSelectedCert(): Promise<number[] | null> {
    const file = this.input("auth-mtls")?.files?.[0];
    if (!file) {
      return null;
    }
    return Array.from(new Uint8Array(await file.arrayBuffer()));
  }

  private updateCaStatus(byteLength: number): void {
    const status = this.root.getElementById("auth-ca-status");
    if (!status) {
      return;
    }
    status.textContent = byteLength > 0 ? `${t("iam.ca.loaded.prefix")} (${byteLength} ${t("iam.ca.bytes")}).` : t("iam.ca.none");
  }

  private syncAuthUrls(sourceId: string, targetId: string): void {
    const source = this.input(sourceId);
    const target = this.input(targetId);
    if (!source || !target || target.value === source.value) {
      return;
    }
    target.value = source.value;
  }

  private signupUrl(): string {
    return this.value("signup-auth-url") || this.value("auth-url");
  }

  private showError(message: string): void {
    const error = this.root.getElementById("auth-error");
    if (!error) {
      return;
    }
    error.textContent = message;
    error.classList.remove("hidden");
  }

  private hideError(): void {
    this.root.getElementById("auth-error")?.classList.add("hidden");
  }

  private notify(type: "success" | "error", message: string): void {
    window.dispatchEvent(new CustomEvent("toast", { detail: { type, message } }));
  }

  private value(id: string): string {
    return this.input(id)?.value.trim() ?? "";
  }

  private panel(): HTMLElement {
    return this.root.getElementById("iam-panel") as HTMLElement;
  }

  private form(id: string): HTMLFormElement | null {
    return this.root.getElementById(id) as HTMLFormElement | null;
  }

  private input(id: string): HTMLInputElement | null {
    return this.root.getElementById(id) as HTMLInputElement | null;
  }

  private button(id: string): HTMLButtonElement | null {
    return this.root.getElementById(id) as HTMLButtonElement | null;
  }

  private mode(): "auth" | "admin" {
    return this.getAttribute("mode") === "admin" ? "admin" : "auth";
  }

  private clearPasswordFields(): void {
    const password = this.input("auth-password");
    if (password) {
      password.value = "";
    }
  }

  private clearSignupPasswordFields(): void {
    for (const id of ["signup-password", "signup-confirm-password"]) {
      const input = this.input(id);
      if (input) {
        input.value = "";
      }
    }
  }
}

customElements.define("tachyon-iam", TachyonIAM);

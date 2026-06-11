import gsap from "gsap";
import QRCode from "qrcode";

import "./TachyonAuthStepCredentials";
import type { CredentialsSubmittedDetail } from "./TachyonAuthStepCredentials";
import stylesheetText from "../../style.css?inline";
import { trapFocus } from "../../utils/a11y";
import { t } from "../../utils/i18n";
import { resilientInvoke as invoke } from "../../utils/network";

type TachyonAuthStepCredentialsElement = HTMLElement & {
  setUrl: (url: string) => void;
  persistIfRemember: (url: string, username: string, password: string, cert: number[] | null) => Promise<void>;
};

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
  private stagedCredentials: CredentialsSubmittedDetail | null = null;
  private stageSignupSubmitting = false;
  private finalizeSignupSubmitting = false;

  constructor() {
    super();
    this.root = this.attachShadow({ mode: "open" });
    this.root.adoptedStyleSheets = [iamStylesheet];
  }

  connectedCallback(): void {
    this.render();
    this.bindEvents();
    if (this.mode() === "auth") {
      const panel = this.panel();
      if (panel) {
        // Escape closes the auth dialog by hiding the whole component.
        trapFocus(panel, () => {
          void gsap.to(panel, { y: -12, opacity: 0, duration: 0.2 });
          this.classList.add("hidden");
        });
      }
      void gsap.fromTo(panel, { y: 16, opacity: 0 }, { y: 0, opacity: 1, duration: 0.35 });
    }
  }

  private render(): void {
    if (this.mode() === "admin") {
      this.renderAdminPanel();
      return;
    }
    this.root.innerHTML = `
      <section class="fixed inset-0 z-[100] bg-slate-950/95 backdrop-blur-xl flex items-center justify-center text-slate-300">
        <div id="iam-panel" role="dialog" aria-modal="true" aria-labelledby="iam-dialog-title" class="bg-slate-900 border border-slate-800 p-8 rounded-2xl w-full max-w-2xl shadow-2xl relative overflow-hidden">
          <div class="absolute top-0 left-0 w-full h-1 bg-gradient-to-r from-cyan-600 to-blue-500"></div>
          <h2 id="iam-dialog-title" class="text-white text-2xl font-bold mb-1">${t("iam.title")}</h2>
          <p class="text-slate-400 text-sm mb-6">${t("iam.subtitle")}</p>
          <div id="auth-step-login" class="auth-step space-y-3">
            <auth-step-credentials id="cred-step"></auth-step-credentials>
            <button type="button" id="btn-open-signup" class="w-full bg-slate-800 hover:bg-slate-700 py-3 rounded-xl text-white font-semibold transition-all border border-slate-700">${t("iam.register")}</button>
          </div>
          <form id="auth-step-mfa" class="auth-step hidden space-y-4" aria-label="${t("iam.mfa.form-label")}">
            <div class="rounded-xl border border-cyan-500/20 bg-cyan-500/5 px-4 py-3 text-sm text-cyan-200" role="status">${t("iam.mfa.prompt")}</div>
            <label for="auth-mfa-code" class="sr-only">${t("iam.mfa.code-label")}</label>
            <input type="text" id="auth-mfa-code" placeholder="000000" maxlength="6" aria-required="true" aria-label="${t("iam.mfa.code-label")}" autocomplete="one-time-code" inputmode="numeric" pattern="[0-9]{6}" class="w-full bg-slate-950 border border-slate-700 p-4 rounded-lg text-cyan-400 text-center text-3xl tracking-[0.35em] font-mono focus:border-cyan-500 focus:outline-none" />
            <button id="btn-mfa-submit" class="w-full bg-cyan-600 hover:bg-cyan-500 py-3 rounded-xl text-white font-bold transition-all">${t("iam.mfa.verify")}</button>
          </form>
          <form id="auth-step-signup-token" class="auth-step hidden space-y-4" aria-label="${t("iam.signup.form-label")}">
            <div class="rounded-xl border border-cyan-500/20 bg-cyan-500/5 px-4 py-3 text-sm text-cyan-200">${t("iam.signup.intro")}</div>
            <label for="signup-auth-url" class="sr-only">${t("iam.placeholder.url")}</label>
            <input type="text" id="signup-auth-url" placeholder="${t("iam.placeholder.url")}" aria-required="true" class="w-full bg-slate-950 border border-slate-700 p-3 rounded-lg text-white text-sm font-mono" />
            <label for="signup-token" class="sr-only">${t("iam.placeholder.invite")}</label>
            <input type="password" id="signup-token" placeholder="${t("iam.placeholder.invite")}" aria-required="true" class="w-full bg-slate-950 border border-slate-700 p-3 rounded-lg text-white text-sm font-mono" />
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
              <button id="btn-signup-profile-submit" class="flex w-full items-center justify-center gap-2 bg-cyan-600 hover:bg-cyan-500 py-3 rounded-xl text-white font-bold transition-all">
                <span id="signup-stage-spinner" class="hidden h-4 w-4 animate-spin rounded-full border-2 border-white/30 border-t-white" aria-hidden="true"></span>
                <span id="signup-stage-label">${t("iam.signup.stage-account")}</span>
              </button>
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
              <button id="btn-signup-totp-submit" class="flex w-full items-center justify-center gap-2 bg-cyan-600 hover:bg-cyan-500 py-3 rounded-xl text-white font-bold transition-all">
                <span id="signup-finalize-spinner" class="hidden h-4 w-4 animate-spin rounded-full border-2 border-white/30 border-t-white" aria-hidden="true"></span>
                <span id="signup-finalize-label">${t("iam.signup.finalize")}</span>
              </button>
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
    this.root.getElementById("cred-step")?.addEventListener("credentials:submitted", (event) => {
      this.stagedCredentials = (event as CustomEvent<CredentialsSubmittedDetail>).detail;
      void this.login();
    });
    this.root.getElementById("cred-step")?.addEventListener("credentials:url-changed", (event) => {
      const url = (event as CustomEvent<{ url: string }>).detail.url;
      const target = this.input("signup-auth-url");
      if (target && target.value !== url) target.value = url;
    });
    this.input("signup-auth-url")?.addEventListener("input", () => {
      const url = this.value("signup-auth-url");
      this.credentialsStep()?.setUrl(url);
    });
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
        // eslint-disable-next-line @typescript-eslint/no-unsafe-call, @typescript-eslint/no-unsafe-member-access
        const dataUrl = await (QRCode as unknown as { toDataURL: (text: string, opts: object) => Promise<string> }).toDataURL(invite.qrPayload, {
          margin: 1,
          width: 180,
          color: { dark: "#020617", light: "#ffffff" },
        });
        const img = document.createElement("img");
        img.src = dataUrl;
        img.alt = "Enrollment QR code";
        img.width = 180;
        qr.replaceChildren(img);
      }
      container?.classList.remove("hidden");
      container?.classList.add("flex");
      this.notify("success", t("iam.admin.invite-generated"));
    } catch (error) {
      this.notify("error", error instanceof Error ? error.message : String(error));
    }
  }

  private async login(): Promise<void> {
    const creds = this.stagedCredentials;
    if (!creds?.url || !creds.username || !creds.password) {
      this.showError(t("iam.error.login-required"));
      return;
    }
    try {
      const response = await invoke<AuthLoginResponse>("authn_login", {
        payload: { url: creds.url, username: creds.username, password: creds.password, cert: creds.cert },
      });
      if (!response.sessionId) {
        throw new Error(t("iam.error.no-session"));
      }
      await this.credentialsStep()?.persistIfRemember(creds.url, creds.username, creds.password, creds.cert);
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
          url: this.stagedCredentials?.url ?? "",
          sessionId: this.stagedLogin.sessionId,
          totpCode,
          cert: this.stagedCredentials?.cert ?? null,
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
        payload: { url: this.signupUrl(), token: this.value("signup-token"), cert: this.stagedCredentials?.cert ?? null },
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
    if (this.stageSignupSubmitting) {
      return;
    }
    if (!this.signupUrl()) {
      this.showError(t("iam.error.url-required"));
      return;
    }
    const password = this.value("signup-password");
    if (password.length < 8 || password !== this.value("signup-confirm-password")) {
      this.showError(t("iam.error.password-rules"));
      return;
    }
    this.stageSignupSubmitting = true;
    this.setSignupStageSubmitting(true);
    try {
      this.stagedSignup = await invoke<StagedSignupSession>("stage_signup", {
        payload: {
          url: this.signupUrl(),
          token: this.value("signup-token"),
          firstName: this.value("signup-first-name"),
          lastName: this.value("signup-last-name"),
          username: this.value("signup-username"),
          password,
          cert: this.stagedCredentials?.cert ?? null,
        },
      });
      await this.renderTotpEnrollment(this.stagedSignup);
      this.clearSignupPasswordFields();
      await this.switchStep("signup-totp");
    } catch (error) {
      this.emitError(error, "signup_stage_failed");
    } finally {
      this.stageSignupSubmitting = false;
      this.setSignupStageSubmitting(false);
    }
  }

  private setSignupStageSubmitting(isSubmitting: boolean): void {
    const button = this.button("btn-signup-profile-submit");
    const spinner = this.root.getElementById("signup-stage-spinner");
    const label = this.root.getElementById("signup-stage-label");
    if (!button) {
      return;
    }
    button.disabled = isSubmitting;
    button.setAttribute("aria-busy", String(isSubmitting));
    button.classList.toggle("opacity-60", isSubmitting);
    button.classList.toggle("cursor-not-allowed", isSubmitting);
    spinner?.classList.toggle("hidden", !isSubmitting);
    if (label) {
      label.textContent = isSubmitting
        ? t("iam.signup.stage-account-loading")
        : t("iam.signup.stage-account");
    }
  }

  private async finalizeSignup(): Promise<void> {
    if (this.finalizeSignupSubmitting) {
      return;
    }
    if (!this.stagedSignup) {
      this.showError(t("iam.error.no-staged-signup"));
      return;
    }
    if (!this.signupUrl()) {
      this.showError(t("iam.error.url-required"));
      return;
    }
    const totpCode = this.value("signup-totp-code").replace(/\s+/g, "");
    if (!/^\d{6}$/.test(totpCode)) {
      this.showError(t("iam.error.totp-format"));
      return;
    }
    this.finalizeSignupSubmitting = true;
    this.setSignupFinalizeSubmitting(true);
    try {
      const response = await invoke<AuthLoginResponse>("finalize_signup", {
        payload: {
          url: this.signupUrl(),
          sessionId: this.stagedSignup.sessionId,
          totpCode,
          cert: this.stagedCredentials?.cert ?? null,
        },
      });
      await this.completeAuthentication({
        user: response.username,
        role: this.claims?.roles[0] ?? "admin",
        token: this.value("signup-token"),
      });
    } catch (error) {
      this.emitError(error, "signup_finalize_failed");
    } finally {
      this.finalizeSignupSubmitting = false;
      this.setSignupFinalizeSubmitting(false);
    }
  }

  private setSignupFinalizeSubmitting(isSubmitting: boolean): void {
    const button = this.button("btn-signup-totp-submit");
    const spinner = this.root.getElementById("signup-finalize-spinner");
    const label = this.root.getElementById("signup-finalize-label");
    if (!button) {
      return;
    }
    button.disabled = isSubmitting;
    button.setAttribute("aria-busy", String(isSubmitting));
    button.classList.toggle("opacity-60", isSubmitting);
    button.classList.toggle("cursor-not-allowed", isSubmitting);
    spinner?.classList.toggle("hidden", !isSubmitting);
    if (label) {
      label.textContent = isSubmitting ? t("iam.signup.finalize-loading") : t("iam.signup.finalize");
    }
  }

  private async completeAuthentication(detail: AuthenticatedDetail): Promise<void> {
    this.hideError();
    window.dispatchEvent(new CustomEvent("connection:connected"));
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
    username.value = [first, last]
      .filter(Boolean)
      .join(".")
      .normalize("NFD")
      .replace(/\p{M}/gu, "")
      .replace(/[^a-z0-9._-]/g, "");
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
      // eslint-disable-next-line @typescript-eslint/no-unsafe-call, @typescript-eslint/no-unsafe-member-access
      const dataUrl = await (QRCode as unknown as { toDataURL: (text: string, opts: object) => Promise<string> }).toDataURL(session.provisioningUri, {
        margin: 1,
        width: 256,
        color: { dark: "#e2e8f0", light: "#0f172a" },
      });
      const img = document.createElement("img");
      img.src = dataUrl;
      img.alt = "TOTP provisioning QR code";
      img.width = 256;
      qr.replaceChildren(img);
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

  private credentialsStep(): TachyonAuthStepCredentialsElement | null {
    return this.root.getElementById("cred-step") as TachyonAuthStepCredentialsElement | null;
  }

  private signupUrl(): string {
    return this.value("signup-auth-url") || this.stagedCredentials?.url || "";
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
    this.stagedCredentials = null;
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

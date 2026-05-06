import gsap from "gsap";
import QRCode from "qrcode";

import stylesheetText from "../../style.css?inline";
import { resilientInvoke as invoke } from "../../utils/network";

type AuthLoginResponse = {
  username: string;
  endpoint: string;
  requiresMfa: boolean;
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

const iamStylesheet = new CSSStyleSheet();
iamStylesheet.replaceSync(stylesheetText);

export class TachyonIAM extends HTMLElement {
  private readonly root: ShadowRoot;
  private activeStep: "login" | "signup-token" | "signup-profile" | "signup-totp" = "login";
  private claims: RegistrationTokenClaims | null = null;
  private stagedSignup: StagedSignupSession | null = null;

  constructor() {
    super();
    this.root = this.attachShadow({ mode: "open" });
    this.root.adoptedStyleSheets = [iamStylesheet];
  }

  connectedCallback(): void {
    this.render();
    this.bindEvents();
    void gsap.fromTo(this.panel(), { y: 16, opacity: 0 }, { y: 0, opacity: 1, duration: 0.35 });
  }

  private render(): void {
    this.root.innerHTML = `
      <section class="fixed inset-0 z-[100] bg-slate-950/95 backdrop-blur-xl flex items-center justify-center text-slate-300">
        <div id="iam-panel" class="bg-slate-900 border border-slate-800 p-8 rounded-2xl w-full max-w-2xl shadow-2xl relative overflow-hidden">
          <div class="absolute top-0 left-0 w-full h-1 bg-gradient-to-r from-cyan-600 to-blue-500"></div>
          <h2 class="text-white text-2xl font-bold mb-1">Tachyon AuthN</h2>
          <p class="text-slate-400 text-sm mb-6">Zero-Trust Control Plane Access</p>
          <form id="auth-step-login" class="auth-step space-y-3">
            <input type="text" id="auth-url" placeholder="Mesh Node URL (https://...)" class="w-full bg-slate-950 border border-slate-700 p-3 rounded-lg text-white text-sm font-mono" />
            <input type="text" id="auth-username" placeholder="Username" class="w-full bg-slate-950 border border-slate-700 p-3 rounded-lg text-white text-sm" />
            <input type="password" id="auth-password" placeholder="Password" class="w-full bg-slate-950 border border-slate-700 p-3 rounded-lg text-white text-sm" />
            <input type="file" id="auth-mtls" class="w-full text-slate-400 text-sm file:mr-4 file:py-2 file:px-4 file:rounded-lg file:border-0 file:bg-cyan-500/10 file:text-cyan-400 cursor-pointer hover:file:bg-cyan-500/20" />
            <div class="grid gap-3 md:grid-cols-2 pt-3">
              <button id="btn-login-submit" class="w-full bg-cyan-600 hover:bg-cyan-500 py-3 rounded-xl text-white font-bold transition-all shadow-[0_0_15px_rgba(34,211,238,0.2)]">Authenticate</button>
              <button type="button" id="btn-open-signup" class="w-full bg-slate-800 hover:bg-slate-700 py-3 rounded-xl text-white font-semibold transition-all border border-slate-700">Register with Invite Token</button>
            </div>
          </form>
          <form id="auth-step-signup-token" class="auth-step hidden space-y-4">
            <div class="rounded-xl border border-cyan-500/20 bg-cyan-500/5 px-4 py-3 text-sm text-cyan-200">Validate a 24-hour invite token before creating the admin profile.</div>
            <input type="password" id="signup-token" placeholder="Paste invite token" class="w-full bg-slate-950 border border-slate-700 p-3 rounded-lg text-white text-sm font-mono" />
            <div class="grid gap-3 md:grid-cols-2">
              <button type="button" id="btn-signup-token-back" class="w-full bg-slate-800 hover:bg-slate-700 py-3 rounded-xl text-white font-semibold transition-all border border-slate-700">Back to Login</button>
              <button id="btn-signup-token-submit" class="w-full bg-cyan-600 hover:bg-cyan-500 py-3 rounded-xl text-white font-bold transition-all">Validate Invite</button>
            </div>
          </form>
          <form id="auth-step-signup-profile" class="auth-step hidden space-y-3">
            <div class="grid gap-3 md:grid-cols-2">
              <input type="text" id="signup-first-name" placeholder="First Name" class="w-full bg-slate-950 border border-slate-700 p-3 rounded-lg text-white text-sm" />
              <input type="text" id="signup-last-name" placeholder="Last Name" class="w-full bg-slate-950 border border-slate-700 p-3 rounded-lg text-white text-sm" />
            </div>
            <input type="text" id="signup-username" placeholder="Username (auto: firstname.lastname)" readonly class="w-full bg-slate-950 border border-slate-700 p-3 rounded-lg text-slate-300 text-sm font-mono cursor-not-allowed" />
            <input type="password" id="signup-password" placeholder="Password (8+ characters)" class="w-full bg-slate-950 border border-slate-700 p-3 rounded-lg text-white text-sm" />
            <input type="password" id="signup-confirm-password" placeholder="Confirm password" class="w-full bg-slate-950 border border-slate-700 p-3 rounded-lg text-white text-sm" />
            <p id="signup-token-summary" class="text-xs text-slate-500">No invite loaded yet.</p>
            <div class="grid gap-3 md:grid-cols-2 pt-2">
              <button type="button" id="btn-signup-profile-back" class="w-full bg-slate-800 hover:bg-slate-700 py-3 rounded-xl text-white font-semibold transition-all border border-slate-700">Back</button>
              <button id="btn-signup-profile-submit" class="w-full bg-cyan-600 hover:bg-cyan-500 py-3 rounded-xl text-white font-bold transition-all">Stage Account</button>
            </div>
          </form>
          <form id="auth-step-signup-totp" class="auth-step hidden space-y-4">
            <div class="grid gap-4 lg:grid-cols-[0.9fr_1.1fr]">
              <div class="rounded-2xl border border-slate-800 bg-slate-950/70 p-5">
                <div class="mb-2 text-xs uppercase tracking-[0.2em] text-slate-500">Authenticator QR</div>
                <div id="signup-totp-qr" class="flex min-h-64 items-center justify-center rounded-2xl border border-dashed border-slate-700 bg-slate-900 p-4 text-center text-sm text-slate-500">
                  Waiting for staged enrollment.
                </div>
              </div>
              <div class="rounded-2xl border border-slate-800 bg-slate-950/70 p-5 space-y-4">
                <div>
                  <div class="mb-2 text-xs uppercase tracking-[0.2em] text-slate-500">Session</div>
                  <div id="signup-session-id" class="break-all font-mono text-sm text-cyan-300">Pending</div>
                </div>
                <div>
                  <div class="mb-2 text-xs uppercase tracking-[0.2em] text-slate-500">Manual Secret</div>
                  <div id="signup-manual-secret" class="break-all font-mono text-sm text-white">Scan the QR code when available.</div>
                </div>
              </div>
            </div>
            <input type="text" id="signup-totp-code" placeholder="000000" maxlength="6" class="w-full bg-slate-950 border border-slate-700 p-4 rounded-lg text-cyan-400 text-center text-3xl tracking-[0.35em] font-mono focus:border-cyan-500 focus:outline-none" />
            <div class="grid gap-3 md:grid-cols-2">
              <button type="button" id="btn-signup-totp-back" class="w-full bg-slate-800 hover:bg-slate-700 py-3 rounded-xl text-white font-semibold transition-all border border-slate-700">Back</button>
              <button id="btn-signup-totp-submit" class="w-full bg-cyan-600 hover:bg-cyan-500 py-3 rounded-xl text-white font-bold transition-all">Finalize Enrollment</button>
            </div>
          </form>
          <div id="auth-error" class="hidden mt-4 rounded-lg border border-red-500/20 bg-red-500/10 px-3 py-2 text-xs text-red-300 text-center">Authentication failed.</div>
        </div>
      </section>
    `;
  }

  private bindEvents(): void {
    this.form("auth-step-login")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.login();
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

  private async login(): Promise<void> {
    const url = this.value("auth-url");
    const username = this.value("auth-username");
    const password = this.value("auth-password");
    if (!url || !username || !password) {
      this.showError("Node URL, username and password are required.");
      return;
    }
    try {
      const response = await invoke<AuthLoginResponse>("authn_login", {
        payload: { url, username, password, cert: null },
      });
      if (response.requiresMfa) {
        this.showError("MFA challenge is not available in this component yet.");
        return;
      }
      await this.completeAuthentication({ user: response.username, role: "admin", token: password });
    } catch (error) {
      this.emitError(error, "authn_login_failed");
    }
  }

  private async validateInvite(): Promise<void> {
    try {
      this.claims = await invoke<RegistrationTokenClaims>("validate_signup_token", {
        payload: { url: this.value("auth-url"), token: this.value("signup-token"), cert: null },
      });
      const summary = this.root.getElementById("signup-token-summary");
      if (summary) {
        summary.textContent = `${this.claims.subject} | roles=${this.claims.roles.join(", ") || "none"} | scopes=${this.claims.scopes.join(", ") || "none"}`;
      }
      await this.switchStep("signup-profile");
    } catch (error) {
      this.emitError(error, "signup_token_failed");
    }
  }

  private async stageAccount(): Promise<void> {
    const password = this.value("signup-password");
    if (password.length < 8 || password !== this.value("signup-confirm-password")) {
      this.showError("Password must be at least 8 characters and match confirmation.");
      return;
    }
    try {
      this.stagedSignup = await invoke<StagedSignupSession>("stage_signup", {
        payload: {
          url: this.value("auth-url"),
          token: this.value("signup-token"),
          firstName: this.value("signup-first-name"),
          lastName: this.value("signup-last-name"),
          username: this.value("signup-username"),
          password,
          cert: null,
        },
      });
      await this.renderTotpEnrollment(this.stagedSignup);
      await this.switchStep("signup-totp");
    } catch (error) {
      this.emitError(error, "signup_stage_failed");
    }
  }

  private async finalizeSignup(): Promise<void> {
    if (!this.stagedSignup) {
      this.showError("No staged signup session is active.");
      return;
    }
    try {
      const response = await invoke<AuthLoginResponse>("finalize_signup", {
        payload: {
          url: this.value("auth-url"),
          sessionId: this.stagedSignup.sessionId,
          totpCode: this.value("signup-totp-code"),
          cert: null,
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
      qr.textContent = `Unable to render QR code. Manual secret: ${this.extractProvisioningSecret(session.provisioningUri)}`;
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
}

customElements.define("tachyon-iam", TachyonIAM);

import gsap from "gsap";
import QRCode from "qrcode";

import stylesheetText from "../../style.css?inline";
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

const iamStylesheet = new CSSStyleSheet();
iamStylesheet.replaceSync(stylesheetText);
const savedCredentialsKey = "tachyon:auth:saved-credentials";

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
    void gsap.fromTo(this.panel(), { y: 16, opacity: 0 }, { y: 0, opacity: 1, duration: 0.35 });
  }

  private render(): void {
    this.root.innerHTML = `
      <section class="fixed inset-0 z-[100] bg-slate-950/95 backdrop-blur-xl flex items-center justify-center text-slate-300">
        <div id="iam-panel" class="bg-slate-900 border border-slate-800 p-8 rounded-2xl w-full max-w-2xl shadow-2xl relative overflow-hidden">
          <div class="absolute top-0 left-0 w-full h-1 bg-gradient-to-r from-cyan-600 to-blue-500"></div>
          <h2 class="text-white text-2xl font-bold mb-1">Tachyon AuthN</h2>
          <p class="text-slate-400 text-sm mb-6">Zero-Trust Control Plane Access</p>
          <form id="iam-signup-form" class="mb-6 space-y-6 max-w-lg rounded-xl border border-slate-800 bg-slate-900/50 p-6 backdrop-blur-sm">
            <div class="border-b border-slate-800 pb-4 mb-4">
              <h3 class="text-lg font-medium text-cyan-400">Stage New Operator</h3>
              <p class="text-sm text-slate-500">Initiate a secure enrollment session</p>
            </div>

            <div class="space-y-2">
              <label class="text-xs font-semibold uppercase tracking-wider text-slate-400">Node URL</label>
              <input type="text" id="iam-url" placeholder="https://..." class="w-full rounded-md border border-slate-700 bg-slate-950 px-3 py-2 text-sm font-mono text-slate-200 focus:border-cyan-500 focus:outline-none focus:ring-1 focus:ring-cyan-500" required>
            </div>

            <div class="grid grid-cols-2 gap-4">
              <div class="space-y-2">
                <label class="text-xs font-semibold uppercase tracking-wider text-slate-400">First Name</label>
                <input type="text" id="iam-first-name" class="w-full rounded-md border border-slate-700 bg-slate-950 px-3 py-2 text-sm text-slate-200 focus:border-cyan-500 focus:outline-none focus:ring-1 focus:ring-cyan-500" required>
              </div>
              <div class="space-y-2">
                <label class="text-xs font-semibold uppercase tracking-wider text-slate-400">Last Name</label>
                <input type="text" id="iam-last-name" class="w-full rounded-md border border-slate-700 bg-slate-950 px-3 py-2 text-sm text-slate-200 focus:border-cyan-500 focus:outline-none focus:ring-1 focus:ring-cyan-500" required>
              </div>
            </div>

            <div class="space-y-2">
              <label class="text-xs font-semibold uppercase tracking-wider text-slate-400">Username</label>
              <input type="text" id="iam-username" class="w-full rounded-md border border-slate-700 bg-slate-950 px-3 py-2 text-sm text-slate-200 focus:border-cyan-500 focus:outline-none focus:ring-1 focus:ring-cyan-500" required>
            </div>

            <div class="space-y-2">
              <label class="text-xs font-semibold uppercase tracking-wider text-slate-400">Initial Password</label>
              <input type="password" id="iam-password" class="w-full rounded-md border border-slate-700 bg-slate-950 px-3 py-2 text-sm text-slate-200 focus:border-cyan-500 focus:outline-none focus:ring-1 focus:ring-cyan-500" required>
            </div>

            <div class="space-y-2">
              <label class="text-xs font-semibold uppercase tracking-wider text-slate-400">Enrollment Token</label>
              <input type="text" id="iam-token" class="w-full rounded-md border border-slate-700 bg-slate-950 px-3 py-2 text-sm font-mono text-cyan-300 focus:border-cyan-500 focus:outline-none focus:ring-1 focus:ring-cyan-500" required>
            </div>

            <div class="pt-4">
              <button type="submit" class="w-full rounded-md bg-cyan-500/20 hover:bg-cyan-500/30 border border-cyan-500/50 px-4 py-2.5 text-sm font-medium text-cyan-300 transition-colors focus:outline-none focus:ring-2 focus:ring-cyan-500 focus:ring-offset-2 focus:ring-offset-slate-900">
                Provision Identity
              </button>
            </div>
          </form>
          <form id="auth-step-login" class="auth-step space-y-3">
            <input type="text" id="auth-url" placeholder="Mesh Node URL (https://...)" class="w-full bg-slate-950 border border-slate-700 p-3 rounded-lg text-white text-sm font-mono" />
            <input type="text" id="auth-username" placeholder="Username" class="w-full bg-slate-950 border border-slate-700 p-3 rounded-lg text-white text-sm" />
            <div class="flex gap-2">
              <input type="password" id="auth-password" placeholder="Password" class="min-w-0 flex-1 bg-slate-950 border border-slate-700 p-3 rounded-lg text-white text-sm" />
              <button type="button" id="btn-toggle-login-password" class="rounded-lg border border-slate-700 bg-slate-800 px-3 text-sm text-slate-300">Show</button>
            </div>
            <label class="flex items-center gap-3 rounded-lg border border-slate-800 bg-slate-950/70 px-3 py-2 text-xs text-slate-300">
              <input type="checkbox" id="remember-credentials" class="h-4 w-4 accent-cyan-500" />
              <span>Remember login and password on this workstation</span>
            </label>
            <input type="file" id="auth-mtls" class="w-full text-slate-400 text-sm file:mr-4 file:py-2 file:px-4 file:rounded-lg file:border-0 file:bg-cyan-500/10 file:text-cyan-400 cursor-pointer hover:file:bg-cyan-500/20" />
            <div class="grid gap-3 md:grid-cols-2 pt-3">
              <button id="btn-login-submit" class="w-full bg-cyan-600 hover:bg-cyan-500 py-3 rounded-xl text-white font-bold transition-all shadow-[0_0_15px_rgba(34,211,238,0.2)]">Authenticate</button>
              <button type="button" id="btn-open-signup" class="w-full bg-slate-800 hover:bg-slate-700 py-3 rounded-xl text-white font-semibold transition-all border border-slate-700">Register with Invite Token</button>
            </div>
          </form>
          <form id="auth-step-mfa" class="auth-step hidden space-y-4">
            <div class="rounded-xl border border-cyan-500/20 bg-cyan-500/5 px-4 py-3 text-sm text-cyan-200">Enter the 6-digit code from your authenticator app.</div>
            <input type="text" id="auth-mfa-code" placeholder="000000" maxlength="6" class="w-full bg-slate-950 border border-slate-700 p-4 rounded-lg text-cyan-400 text-center text-3xl tracking-[0.35em] font-mono focus:border-cyan-500 focus:outline-none" />
            <button id="btn-mfa-submit" class="w-full bg-cyan-600 hover:bg-cyan-500 py-3 rounded-xl text-white font-bold transition-all">Verify Code</button>
          </form>
          <form id="auth-step-signup-token" class="auth-step hidden space-y-4">
            <div class="rounded-xl border border-cyan-500/20 bg-cyan-500/5 px-4 py-3 text-sm text-cyan-200">Validate a 24-hour invite token before creating the admin profile.</div>
            <input type="text" id="signup-auth-url" placeholder="Mesh Node URL (https://...)" class="w-full bg-slate-950 border border-slate-700 p-3 rounded-lg text-white text-sm font-mono" />
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
    this.form("iam-signup-form")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.stageOperator();
    });
    this.form("auth-step-login")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.login();
    });
    this.button("btn-toggle-login-password")?.addEventListener("click", () => this.togglePassword("auth-password"));
    this.input("remember-credentials")?.addEventListener("change", () => this.persistCredentialsPreference());
    this.input("iam-url")?.addEventListener("input", () => this.syncAuthUrls("iam-url", "auth-url"));
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
    this.restoreSavedCredentials();
  }

  private async stageOperator(): Promise<void> {
    const url = this.value("iam-url") || this.signupUrl();
    const token = this.value("iam-token");
    const firstName = this.value("iam-first-name");
    const lastName = this.value("iam-last-name");
    const username = this.value("iam-username");
    const password = this.value("iam-password");
    if (!url || !token || !firstName || !lastName || !username || !password) {
      this.notify("error", "Node URL, token, first name, last name, username and password are required.");
      return;
    }
    try {
      await invoke<StagedSignupSession>("stage_signup", {
        payload: { url, token, firstName, lastName, username, password, cert: null },
      });
      this.notify("success", `Enrollment staged for ${username}.`);
    } catch (error) {
      this.notify("error", error instanceof Error ? error.message : String(error));
    }
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
        if (!response.sessionId) {
          throw new Error("Login staged without an MFA session id.");
        }
        this.persistCredentialsPreference();
        this.stagedLogin = response;
        await this.switchStep("mfa");
        return;
      }
      this.persistCredentialsPreference();
      await this.completeAuthentication({ user: response.username, role: "admin", token: password });
    } catch (error) {
      this.emitError(error, "authn_login_failed");
    }
  }

  private async finalizeLogin(): Promise<void> {
    if (!this.stagedLogin?.sessionId) {
      this.showError("No staged login session is active.");
      return;
    }
    const totpCode = this.value("auth-mfa-code").replace(/\s+/g, "");
    if (!/^\d{6}$/.test(totpCode)) {
      this.showError("Enter a 6-digit MFA code.");
      return;
    }
    try {
      const response = await invoke<AuthLoginResponse>("finalize_login", {
        payload: {
          url: this.value("auth-url"),
          sessionId: this.stagedLogin.sessionId,
          totpCode,
          cert: null,
        },
      });
      this.stagedLogin = null;
      await this.completeAuthentication({ user: response.username, role: "admin", token: "" });
    } catch (error) {
      this.emitError(error, "authn_mfa_failed");
    }
  }

  private async validateInvite(): Promise<void> {
    if (!this.signupUrl()) {
      this.showError("Mesh Node URL is required for enrollment.");
      return;
    }
    try {
      this.claims = await invoke<RegistrationTokenClaims>("validate_signup_token", {
        payload: { url: this.signupUrl(), token: this.value("signup-token"), cert: null },
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
    if (!this.signupUrl()) {
      this.showError("Mesh Node URL is required for enrollment.");
      return;
    }
    const password = this.value("signup-password");
    if (password.length < 8 || password !== this.value("signup-confirm-password")) {
      this.showError("Password must be at least 8 characters and match confirmation.");
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
    if (!this.signupUrl()) {
      this.showError("Mesh Node URL is required for enrollment.");
      return;
    }
    try {
      const response = await invoke<AuthLoginResponse>("finalize_signup", {
        payload: {
          url: this.signupUrl(),
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

  private togglePassword(id: string): void {
    const input = this.input(id);
    const button = this.button("btn-toggle-login-password");
    if (!input) {
      return;
    }
    input.type = input.type === "password" ? "text" : "password";
    if (button) {
      button.textContent = input.type === "password" ? "Show" : "Hide";
    }
  }

  private restoreSavedCredentials(): void {
    try {
      const raw = localStorage.getItem(savedCredentialsKey);
      if (!raw) {
        return;
      }
      const parsed = JSON.parse(raw) as Partial<{ url: string; username: string; password: string }>;
      if (parsed.url) {
        const iamUrl = this.input("iam-url");
        if (iamUrl) {
          iamUrl.value = parsed.url;
        }
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
      localStorage.removeItem(savedCredentialsKey);
    }
  }

  private persistCredentialsPreference(): void {
    if (!this.input("remember-credentials")?.checked) {
      localStorage.removeItem(savedCredentialsKey);
      return;
    }
    localStorage.setItem(
      savedCredentialsKey,
      JSON.stringify({
        url: this.value("auth-url"),
        username: this.value("auth-username"),
        password: this.input("auth-password")?.value ?? "",
      }),
    );
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
}

customElements.define("tachyon-iam", TachyonIAM);

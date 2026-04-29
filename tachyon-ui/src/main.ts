import { listen } from "@tauri-apps/api/event";
import gsap from "gsap";
import QRCode from "qrcode";

import { mountNetworkStatus } from "./components/NetworkStatus";
import { resilientInvoke as invoke } from "./utils/network";

const configuredNodeUrl = (import.meta.env.VITE_TACHYON_NODE_URL ?? "").trim();
const configuredNodeToken = (import.meta.env.VITE_TACHYON_NODE_TOKEN ?? "").trim();

type ViewName = "dashboard" | "topology" | "registry" | "identity" | "account" | "resources" | "broker";

type MeshResourceKind = "internal" | "external";

type MeshResource = {
  name: string;
  type: MeshResourceKind;
  target: string;
  pending: boolean;
  allowedMethods?: string[];
  versionConstraint?: string;
};

type MeshResourceInput = {
  name: string;
  type: MeshResourceKind;
  target: string;
  allowedMethods?: string[];
  versionConstraint?: string;
};

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

type IamUserSummary = {
  username: string;
  groups: string[];
  securityStatus: string;
};

type MeshRouteSummary = {
  path: string;
  name: string;
  role: string;
  targetCount: number;
};

type MeshGraphSnapshot = {
  source: string;
  status: string;
  routes: MeshRouteSummary[];
  batchTargets: string[];
};

document.addEventListener("DOMContentLoaded", () => {
  const refreshBtn = document.getElementById("refresh-btn");
  const activeFaaS = document.getElementById("active-faas");
  const peerCount = document.getElementById("peer-count");
  const nodeId = document.getElementById("node-id");
  const headerTitle = document.getElementById("header-title");
  const headerSubtitle = document.getElementById("header-subtitle");
  const header = document.querySelector<HTMLElement>(".header");
  const overlay = document.getElementById("auth-overlay");
  const firstRunModal = document.getElementById("first-run-modal");
  const authLoginStep = document.getElementById("auth-step-login");
  const authMfaStep = document.getElementById("auth-step-mfa");
  const signupTokenStep = document.getElementById("auth-step-signup-token");
  const signupProfileStep = document.getElementById("auth-step-signup-profile");
  const signupTotpStep = document.getElementById("auth-step-signup-totp");
  const nodeUrl = document.getElementById("auth-url") as HTMLInputElement | null;
  const authUsername = document.getElementById("auth-username") as HTMLInputElement | null;
  const nodeToken = document.getElementById("auth-password") as HTMLInputElement | null;
  const mtlsFile = document.getElementById("auth-mtls") as HTMLInputElement | null;
  const connectSubmitBtn = document.getElementById("btn-login-submit") as HTMLButtonElement | null;
  const openSignupBtn = document.getElementById("btn-open-signup") as HTMLButtonElement | null;
  const backToLoginBtn = document.getElementById("btn-back-to-login") as HTMLButtonElement | null;
  const authMfaCode = document.getElementById("auth-mfa-code") as HTMLInputElement | null;
  const authMfaSubmitBtn = document.getElementById("btn-mfa-submit") as HTMLButtonElement | null;
  const signupTokenInput = document.getElementById("signup-token") as HTMLInputElement | null;
  const signupTokenBackBtn = document.getElementById("btn-signup-token-back") as HTMLButtonElement | null;
  const signupTokenSubmitBtn = document.getElementById("btn-signup-token-submit") as HTMLButtonElement | null;
  const signupFirstName = document.getElementById("signup-first-name") as HTMLInputElement | null;
  const signupLastName = document.getElementById("signup-last-name") as HTMLInputElement | null;
  const signupUsername = document.getElementById("signup-username") as HTMLInputElement | null;
  const signupPassword = document.getElementById("signup-password") as HTMLInputElement | null;
  const signupConfirmPassword = document.getElementById("signup-confirm-password") as HTMLInputElement | null;
  const signupPasswordError = document.getElementById("signup-password-error");
  const signupProfileBackBtn = document.getElementById("btn-signup-profile-back") as HTMLButtonElement | null;
  const signupProfileSubmitBtn = document.getElementById("btn-signup-profile-submit") as HTMLButtonElement | null;
  const signupTokenSubject = document.getElementById("signup-token-subject");
  const signupTokenRoles = document.getElementById("signup-token-roles");
  const signupTokenScopes = document.getElementById("signup-token-scopes");
  const signupTokenExpiry = document.getElementById("signup-token-expiry");
  const signupTotpQr = document.getElementById("signup-totp-qr");
  const signupSessionId = document.getElementById("signup-session-id");
  const signupManualSecret = document.getElementById("signup-manual-secret");
  const signupSessionExpiry = document.getElementById("signup-session-expiry");
  const signupTotpCode = document.getElementById("signup-totp-code") as HTMLInputElement | null;
  const signupTotpBackBtn = document.getElementById("btn-signup-totp-back") as HTMLButtonElement | null;
  const signupTotpSubmitBtn = document.getElementById("btn-signup-totp-submit") as HTMLButtonElement | null;
  const connectionError = document.getElementById("auth-error");
  const qrStep = document.getElementById("qr-step");
  const recoveryCodesStep = document.getElementById("recovery-codes-step");
  const showRecoveryCodesBtn = document.getElementById("show-recovery-codes-btn") as HTMLButtonElement | null;
  const codesContainer = document.getElementById("codes-container");
  const downloadCodesBtn = document.getElementById("download-codes-btn") as HTMLButtonElement | null;
  const confirmSavedBtn = document.getElementById("confirm-saved-btn") as HTMLButtonElement | null;
  const onboardingError = document.getElementById("onboarding-error");
  const assetUploadInput = document.getElementById("asset-upload") as HTMLInputElement | null;
  const assetUploadBtn = document.getElementById("asset-upload-btn") as HTMLButtonElement | null;
  const assetUploadResult = document.getElementById("asset-upload-result");
  const modelUploadInput = document.getElementById("model-upload") as HTMLInputElement | null;
  const modelUploadBtn = document.getElementById("model-upload-btn") as HTMLButtonElement | null;
  const modelUploadResult = document.getElementById("model-upload-result");
  const modelProgress = document.getElementById("model-progress");
  const viewContainers = document.querySelectorAll(".view-container");
  const navLinks = Array.from(document.querySelectorAll<HTMLAnchorElement>(".nav-link[data-view]"));
  const refreshTopologyBtn = document.getElementById("refresh-topology-btn") as HTMLButtonElement | null;
  const meshGraphSource = document.getElementById("mesh-graph-source");
  const meshGraphStatus = document.getElementById("mesh-graph-status");
  const meshRouteCount = document.getElementById("mesh-route-count");
  const meshRouteList = document.getElementById("mesh-route-list");
  const meshBatchList = document.getElementById("mesh-batch-list");
  const iamUserList = document.getElementById("iam-user-list");
  const identityConnectionSource = document.getElementById("identity-connection-source");
  const identityMfaStatus = document.getElementById("identity-mfa-status");
  const identityRecoveryStatus = document.getElementById("identity-recovery-status");
  const identityMfaBtn = document.getElementById("identity-mfa-btn") as HTMLButtonElement | null;
  const identityActionResult = document.getElementById("identity-action-result");
  const newUserBtn = document.getElementById("btn-new-user") as HTMLButtonElement | null;
  const accountConnectionSource = document.getElementById("account-connection-source");
  const patNameInput = document.getElementById("pat-name") as HTMLInputElement | null;
  const patScopesInput = document.getElementById("pat-scopes") as HTMLInputElement | null;
  const patTtlInput = document.getElementById("pat-ttl") as HTMLSelectElement | null;
  const patGenerateBtn = document.getElementById("btn-generate-pat") as HTMLButtonElement | null;
  const patResult = document.getElementById("pat-result");
  const regen2faBtn = document.getElementById("btn-regen-2fa") as HTMLButtonElement | null;
  const accountSecurityResult = document.getElementById("account-security-result");
  const resourceTableBody = document.getElementById("resource-table-body");
  const resourceEmptyState = document.getElementById("resource-empty-state");
  const resourceActionResult = document.getElementById("resource-action-result");
  const resourceSearchInput = document.getElementById("resource-search") as HTMLInputElement | null;
  const resourceFilterButtons = Array.from(
    document.querySelectorAll<HTMLButtonElement>(".resource-filter[data-resource-filter]"),
  );
  const refreshResourcesBtn = document.getElementById("btn-refresh-resources") as HTMLButtonElement | null;
  const addResourceBtn = document.getElementById("btn-add-resource") as HTMLButtonElement | null;
  const resourceEditorModal = document.getElementById("resource-editor-modal");
  const resourceEditorTitle = document.getElementById("resource-editor-title");
  const resourceNameInput = document.getElementById("resource-name") as HTMLInputElement | null;
  const resourceTargetInput = document.getElementById("resource-target") as HTMLInputElement | null;
  const resourceAllowedMethodsInput = document.getElementById("resource-allowed-methods") as HTMLInputElement | null;
  const resourceVersionConstraintInput = document.getElementById("resource-version-constraint") as HTMLInputElement | null;
  const resourceExternalExtra = document.getElementById("resource-external-extra");
  const resourceInternalExtra = document.getElementById("resource-internal-extra");
  const resourceEditorError = document.getElementById("resource-editor-error");
  const resourceCancelBtn = document.getElementById("btn-resource-cancel") as HTMLButtonElement | null;
  const resourceSaveBtn = document.getElementById("btn-resource-save") as HTMLButtonElement | null;
  const resourceTypeRadios = Array.from(
    document.querySelectorAll<HTMLInputElement>("input[name='resource-type']"),
  );

  let downloadedRecoveryCodes = false;
  let recoveryCodes: string[] = [];
  let activeView: ViewName = "dashboard";
  let activeOperator = "admin";
  let authGatewayValidated = false;
  let iamUsers: IamUserSummary[] = [];
  let activeAuthStep: "login" | "mfa" | "signup-token" | "signup-profile" | "signup-totp" = "login";
  let signupClaims: RegistrationTokenClaims | null = null;
  let stagedSignup: StagedSignupSession | null = null;
  let meshResources: MeshResource[] = [];
  let resourceFilter: "all" | MeshResourceKind = "all";
  let resourceEditorMode: "create" | "edit" = "create";
  let resourceEditorOriginalName: string | null = null;

  const viewPanels: Record<ViewName, HTMLElement | null> = {
    dashboard: document.getElementById("view-dashboard"),
    topology: document.getElementById("view-topology"),
    registry: document.getElementById("view-registry"),
    identity: document.getElementById("view-identity"),
    account: document.getElementById("view-account"),
    resources: document.getElementById("view-resources"),
    broker: document.getElementById("view-broker"),
  };

  const viewMetadata: Record<ViewName, { title: string; subtitle: string }> = {
    dashboard: {
      title: "Local Node Status",
      subtitle: "Observe the connected control plane and shared runtime workflows.",
    },
    topology: {
      title: "Mesh Topology",
      subtitle: "Inspect the sealed route graph and batch surfaces for the active node profile.",
    },
    registry: {
      title: "Asset Registry",
      subtitle: "Publish WebAssembly assets directly into the embedded mesh registry.",
    },
    identity: {
      title: "Identity",
      subtitle: "Review the administrative principal and security onboarding posture.",
    },
    account: {
      title: "My Account",
      subtitle: "Manage personal automation tokens and rotate emergency recovery material.",
    },
    broker: {
      title: "Model Broker",
      subtitle: "Stream large model artifacts into disk-backed storage without RAM spikes.",
    },
    resources: {
      title: "Resource Catalog",
      subtitle: "Visualize and edit logical mesh resources (internal IPC aliases and external HTTPS targets).",
    },
  };

  const tl = gsap.timeline();
  tl.from(".sidebar", { x: -50, opacity: 0, duration: 0.6, ease: "power3.out" })
    .from(".header", { y: -20, opacity: 0, duration: 0.4, ease: "power2.out" }, "-=0.4")
    .from(
      ".stagger-card",
      {
        y: 30,
        opacity: 0,
        duration: 0.6,
        stagger: 0.1,
        ease: "back.out(1.2)",
      },
      "-=0.2",
    );

  if (header) {
    mountNetworkStatus(header);
  }

  if (nodeUrl && !nodeUrl.value && configuredNodeUrl) {
    nodeUrl.value = configuredNodeUrl;
  }
  if (authUsername && !authUsername.value) {
    authUsername.value = activeOperator;
  }
  if (nodeToken && !nodeToken.value && configuredNodeToken) {
    nodeToken.value = configuredNodeToken;
  }

  gsap.to(".pulse-dot", {
    opacity: 0.4,
    scale: 0.8,
    duration: 1.5,
    repeat: -1,
    yoyo: true,
    ease: "sine.inOut",
  });

  const showConnectionError = (message: string) => {
    if (!connectionError) {
      return;
    }

    connectionError.textContent = message;
    connectionError.classList.remove("hidden");
  };

  const clearConnectionError = () => {
    if (!connectionError) {
      return;
    }

    connectionError.textContent = "Authentication failed.";
    connectionError.classList.add("hidden");
  };

  const authSteps = {
    login: authLoginStep,
    mfa: authMfaStep,
    "signup-token": signupTokenStep,
    "signup-profile": signupProfileStep,
    "signup-totp": signupTotpStep,
  } as const;

  const formatUnixTimestamp = (value: number) =>
    new Date(value * 1000).toLocaleString(undefined, {
      year: "numeric",
      month: "short",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
    });

  const renderBadgeList = (
    target: HTMLElement | null,
    values: string[],
    emptyMessage: string,
    tone: "cyan" | "slate" = "cyan",
  ) => {
    if (!target) {
      return;
    }

    if (values.length === 0) {
      target.textContent = emptyMessage;
      target.className = "text-xs text-slate-400";
      return;
    }

    const palette =
      tone === "cyan"
        ? "border-cyan-500/20 bg-cyan-500/10 text-cyan-300"
        : "border-slate-700 bg-slate-900 text-slate-300";
    target.innerHTML = values
      .map(
        (value) =>
          `<span class="inline-flex rounded-full border px-2 py-1 text-[11px] font-medium ${palette}">${value}</span>`,
      )
      .join("");
    target.className = "flex flex-wrap gap-2";
  };

  const extractProvisioningSecret = (provisioningUri: string) => {
    try {
      const parsed = new URL(provisioningUri);
      return parsed.searchParams.get("secret") || provisioningUri;
    } catch {
      return provisioningUri;
    }
  };

  const formatUsername = (first: string, last: string): string => {
    const stripDiacritics = (value: string) =>
      value
        .normalize("NFD")
        .replace(/[̀-ͯ]/g, "")
        .toLowerCase();
    const sanitize = (value: string) =>
      stripDiacritics(value)
        .replace(/\s+/g, ".")
        .replace(/[^a-z0-9._-]/g, "");
    const a = sanitize(first);
    const b = sanitize(last);
    if (!a && !b) {
      return "";
    }
    if (!a) {
      return b;
    }
    if (!b) {
      return a;
    }
    return `${a}.${b}`;
  };

  const recomputeSignupUsername = () => {
    if (!signupUsername) {
      return;
    }
    const computed = formatUsername(
      signupFirstName?.value ?? "",
      signupLastName?.value ?? "",
    );
    signupUsername.value = computed;
  };

  const refreshSignupValidation = () => {
    if (!signupProfileSubmitBtn) {
      return;
    }
    const password = signupPassword?.value ?? "";
    const confirm = signupConfirmPassword?.value ?? "";
    const username = signupUsername?.value ?? "";
    const tooShort = password.length > 0 && password.length < 8;
    const mismatched = password.length > 0 && confirm.length > 0 && password !== confirm;

    if (signupPasswordError) {
      if (tooShort) {
        signupPasswordError.textContent = "Password must be at least 8 characters.";
        signupPasswordError.classList.remove("hidden");
      } else if (mismatched) {
        signupPasswordError.textContent = "Passwords do not match.";
        signupPasswordError.classList.remove("hidden");
      } else {
        signupPasswordError.textContent = "";
        signupPasswordError.classList.add("hidden");
      }
    }

    const ready =
      username.length > 0 &&
      password.length >= 8 &&
      password === confirm;
    signupProfileSubmitBtn.disabled = !ready;
  };

  const togglePasswordVisibility = (button: HTMLElement) => {
    const targetId = button.dataset.passwordToggle;
    if (!targetId) {
      return;
    }
    const input = document.getElementById(targetId) as HTMLInputElement | null;
    if (!input) {
      return;
    }
    const eye = button.querySelector<SVGElement>(".password-eye");
    const eyeOff = button.querySelector<SVGElement>(".password-eye-off");
    if (input.type === "password") {
      input.type = "text";
      eye?.classList.add("hidden");
      eyeOff?.classList.remove("hidden");
    } else {
      input.type = "password";
      eye?.classList.remove("hidden");
      eyeOff?.classList.add("hidden");
    }
  };

  const switchAuthStep = async (
    nextStep: "login" | "mfa" | "signup-token" | "signup-profile" | "signup-totp",
  ) => {
    const currentPanel = authSteps[activeAuthStep];
    const nextPanel = authSteps[nextStep];
    if (!nextPanel || currentPanel === nextPanel) {
      activeAuthStep = nextStep;
      return;
    }

    clearConnectionError();
    if (currentPanel) {
      await gsap.to(currentPanel, {
        autoAlpha: 0,
        y: -12,
        duration: 0.2,
        ease: "power2.inOut",
      });
      currentPanel.classList.add("hidden");
      gsap.set(currentPanel, { clearProps: "all" });
    }

    nextPanel.classList.remove("hidden");
    await gsap.fromTo(
      nextPanel,
      { autoAlpha: 0, y: 18 },
      { autoAlpha: 1, y: 0, duration: 0.24, ease: "power2.out" },
    );
    activeAuthStep = nextStep;
  };

  const resetSignupFlow = () => {
    signupClaims = null;
    stagedSignup = null;
    if (signupTokenInput) {
      signupTokenInput.value = "";
    }
    if (signupFirstName) {
      signupFirstName.value = "";
    }
    if (signupLastName) {
      signupLastName.value = "";
    }
    if (signupUsername) {
      signupUsername.value = "";
    }
    if (signupPassword) {
      signupPassword.value = "";
    }
    if (signupConfirmPassword) {
      signupConfirmPassword.value = "";
    }
    if (signupPasswordError) {
      signupPasswordError.classList.add("hidden");
      signupPasswordError.textContent = "";
    }
    if (signupTotpCode) {
      signupTotpCode.value = "";
    }
    refreshSignupValidation();
    if (signupTokenSubject) {
      signupTokenSubject.textContent = "Awaiting token validation";
    }
    renderBadgeList(signupTokenRoles, [], "No roles loaded yet.");
    renderBadgeList(signupTokenScopes, [], "No scopes loaded yet.", "slate");
    if (signupTokenExpiry) {
      signupTokenExpiry.textContent = "Unknown";
    }
    if (signupTotpQr) {
      signupTotpQr.innerHTML = "Waiting for staged enrollment.";
      signupTotpQr.className =
        "flex min-h-72 items-center justify-center rounded-2xl border border-dashed border-slate-700 bg-slate-900 p-6 text-slate-500";
    }
    if (signupSessionId) {
      signupSessionId.textContent = "Pending";
    }
    if (signupManualSecret) {
      signupManualSecret.textContent = "Scan the QR code when available.";
    }
    if (signupSessionExpiry) {
      signupSessionExpiry.textContent = "Unknown";
    }
  };

  const renderSignupClaims = (claims: RegistrationTokenClaims) => {
    if (signupTokenSubject) {
      signupTokenSubject.textContent = claims.subject;
    }
    renderBadgeList(signupTokenRoles, claims.roles, "No roles assigned.");
    renderBadgeList(signupTokenScopes, claims.scopes, "No scopes assigned.", "slate");
    if (signupTokenExpiry) {
      signupTokenExpiry.textContent = formatUnixTimestamp(claims.expiresAt);
    }
  };

  const renderStagedSignup = async (session: StagedSignupSession) => {
    if (signupSessionId) {
      signupSessionId.textContent = session.sessionId;
    }
    if (signupManualSecret) {
      signupManualSecret.textContent = extractProvisioningSecret(session.provisioningUri);
    }
    if (signupSessionExpiry) {
      signupSessionExpiry.textContent = formatUnixTimestamp(session.expiresAt);
    }
    if (!signupTotpQr) {
      return;
    }

    const svg = await QRCode.toString(session.provisioningUri, {
      type: "svg",
      margin: 1,
      width: 256,
      color: {
        dark: "#e2e8f0",
        light: "#0f172a",
      },
    });
    signupTotpQr.innerHTML = svg;
    signupTotpQr.className =
      "flex min-h-72 items-center justify-center rounded-2xl border border-slate-700 bg-slate-900 p-4";
  };

  const showOnboardingError = (message: string) => {
    if (!onboardingError) {
      return;
    }

    onboardingError.textContent = message;
    onboardingError.classList.remove("hidden");
  };

  const clearOnboardingError = () => {
    if (!onboardingError) {
      return;
    }

    onboardingError.textContent = "Unable to complete security onboarding.";
    onboardingError.classList.add("hidden");
  };

  const onboardingStorageKey = () => {
    const url = nodeUrl?.value.trim() || "default";
    return `tachyon:onboarding:${url}`;
  };

  const updateHeaderForView = (view: ViewName) => {
    const metadata = viewMetadata[view];
    if (headerTitle) {
      headerTitle.textContent = metadata.title;
    }
    if (headerSubtitle) {
      headerSubtitle.textContent = metadata.subtitle;
    }
  };

  const updateNavigationState = (view: ViewName) => {
    navLinks.forEach((link) => {
      const isActive = link.dataset.view === view;
      link.classList.toggle("bg-slate-800", isActive);
      link.classList.toggle("text-cyan-400", isActive);
      link.classList.toggle("font-medium", isActive);
      link.classList.toggle("text-slate-300", !isActive);
    });
  };

  const updateConnectionBadge = () => {
    if (!nodeId) {
      return;
    }

    nodeId.textContent = nodeUrl?.value.trim() || "node-edge-waiting...";
  };

  const renderIdentityView = () => {
    const endpoint = nodeUrl?.value.trim() || "workspace://local";
    const connected = authGatewayValidated;
    const recoveryReady = localStorage.getItem(onboardingStorageKey()) === "complete";
    const renderedUsers =
      iamUsers.length > 0
        ? iamUsers
        : connected
          ? [
              {
                username: activeOperator,
                groups: ["admin", "ops"],
                securityStatus: "Recovery bundle managed through desktop onboarding",
              },
            ]
          : [];

    if (identityConnectionSource) {
      identityConnectionSource.textContent = endpoint;
    }
    if (identityMfaStatus) {
      identityMfaStatus.textContent = connected
        ? recoveryReady
          ? "Recovery bundle secured"
          : "Onboarding required"
        : "Auth gateway locked";
    }
    if (identityRecoveryStatus) {
      identityRecoveryStatus.textContent = recoveryReady
        ? "Downloaded and acknowledged for this endpoint"
        : "Pending download acknowledgement";
    }
    if (identityMfaBtn) {
      identityMfaBtn.disabled = !connected || recoveryReady;
      identityMfaBtn.textContent = !connected
        ? "Authenticate First"
        : recoveryReady
          ? "Recovery Bundle Secured"
          : "Continue Security Setup";
      identityMfaBtn.classList.toggle("opacity-60", recoveryReady);
      identityMfaBtn.classList.toggle("cursor-not-allowed", recoveryReady);
    }
    if (iamUserList) {
      if (renderedUsers.length === 0) {
        iamUserList.innerHTML = `
          <tr>
            <td colspan="4" class="py-6 text-sm text-slate-500">Authenticate through the AuthN gateway to load the active IAM session.</td>
          </tr>
        `;
        return;
      }

      iamUserList.innerHTML = renderedUsers
        .map((user) => {
          const initials = user.username.slice(0, 2).toUpperCase();
          const groupBadges = user.groups
            .map(
              (group) =>
                `<span class="inline-flex rounded-full border border-cyan-500/20 bg-cyan-500/10 px-2 py-1 text-[11px] font-medium text-cyan-300">${group}</span>`,
            )
            .join(" ");
          const securityStatus = recoveryReady ? user.securityStatus : "Onboarding required";

          return `
            <tr class="hover:bg-slate-900/80 transition-colors">
              <td class="py-4 pr-4 text-white">
                <div class="flex items-center gap-3">
                  <div class="flex h-9 w-9 items-center justify-center rounded-full bg-cyan-500/10 text-xs font-semibold text-cyan-300">${initials}</div>
                  <div>
                    <div class="font-medium">${user.username}</div>
                    <div class="text-xs text-slate-500">${connected ? "Active admin session" : "Awaiting auth gateway"}</div>
                  </div>
                </div>
              </td>
              <td class="py-4 pr-4">
                <div class="flex flex-wrap gap-2">${groupBadges}</div>
              </td>
              <td class="py-4 pr-4 text-slate-300">${securityStatus}</td>
              <td class="py-4 text-right">
                <div class="flex items-center justify-end gap-3">
                  <button data-action="roles-hint" class="text-xs font-medium uppercase tracking-wider text-slate-500 hover:text-white transition-colors">RBAC via token scopes</button>
                  <button data-action="regen-mfa" data-username="${user.username}" class="text-xs font-medium uppercase tracking-wider text-red-400 hover:text-red-300 transition-colors">Regen 2FA</button>
                </div>
              </td>
            </tr>
          `;
        })
        .join("");
    }
  };

  const renderAccountView = () => {
    const endpoint = nodeUrl?.value.trim() || "workspace://local";
    if (accountConnectionSource) {
      accountConnectionSource.textContent = endpoint;
    }
  };

  const escapeHtml = (value: string): string =>
    value
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/\"/g, "&quot;")
      .replace(/'/g, "&#39;");

  const showResourceActionResult = (message: string, tone: "ok" | "err" | "info" = "info") => {
    if (!resourceActionResult) {
      return;
    }
    const palette =
      tone === "ok"
        ? "border-emerald-500/30 text-emerald-300"
        : tone === "err"
          ? "border-red-500/30 text-red-400"
          : "border-slate-700 text-slate-300";
    resourceActionResult.className = `mt-4 p-3 bg-slate-900 border ${palette} font-mono text-xs whitespace-pre-wrap break-words rounded-lg`;
    resourceActionResult.textContent = message;
    resourceActionResult.classList.remove("hidden");
  };

  const renderResourceCatalog = () => {
    if (!resourceTableBody) {
      return;
    }
    const search = (resourceSearchInput?.value ?? "").trim().toLowerCase();
    const rows = meshResources.filter((resource) => {
      if (resourceFilter !== "all" && resource.type !== resourceFilter) {
        return false;
      }
      if (!search) {
        return true;
      }
      return (
        resource.name.toLowerCase().includes(search) ||
        resource.target.toLowerCase().includes(search)
      );
    });

    if (rows.length === 0) {
      resourceTableBody.innerHTML = "";
      resourceEmptyState?.classList.remove("hidden");
      return;
    }
    resourceEmptyState?.classList.add("hidden");

    resourceTableBody.innerHTML = rows
      .map((resource) => {
        const typePalette =
          resource.type === "internal"
            ? "border-cyan-500/30 bg-cyan-500/10 text-cyan-300"
            : "border-blue-500/30 bg-blue-500/10 text-blue-300";
        const pendingBadge = resource.pending
          ? `<span title="Requires CLI re-seal of integrity.lock to take effect" class="ml-2 inline-flex items-center rounded-full border border-amber-500/40 bg-amber-500/10 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wider text-amber-300">Pending seal</span>`
          : "";
        const detail =
          resource.type === "external"
            ? `Methods: ${(resource.allowedMethods ?? []).join(", ") || "any"}`
            : resource.versionConstraint
              ? `Version: ${escapeHtml(resource.versionConstraint)}`
              : "—";
        const safeName = escapeHtml(resource.name);
        return `
          <tr class="resource-row hover:bg-slate-900/70 transition-colors">
            <td class="px-4 py-3 font-mono text-cyan-200">${safeName}${pendingBadge}</td>
            <td class="px-4 py-3"><span class="inline-flex rounded-full border ${typePalette} px-2 py-0.5 text-[11px] font-semibold uppercase tracking-wider">${resource.type}</span></td>
            <td class="px-4 py-3 font-mono text-slate-300 break-all">${escapeHtml(resource.target)}</td>
            <td class="px-4 py-3 text-slate-400">${detail}</td>
            <td class="px-4 py-3 text-right">
              <button data-action="resource-edit" data-name="${safeName}" class="text-xs font-medium uppercase tracking-wider text-slate-400 hover:text-cyan-300 transition-colors mr-3">Edit</button>
              <button data-action="resource-delete" data-name="${safeName}" class="text-xs font-medium uppercase tracking-wider text-red-400 hover:text-red-300 transition-colors">Delete</button>
            </td>
          </tr>
        `;
      })
      .join("");

    gsap.from(".resource-row", {
      autoAlpha: 0,
      y: 8,
      duration: 0.25,
      stagger: 0.04,
      ease: "power2.out",
    });
  };

  const loadResources = async () => {
    try {
      const data = await invoke<MeshResource[]>("get_resources");
      meshResources = (data ?? []).map((entry) => ({
        ...entry,
        allowedMethods: entry.allowedMethods ?? [],
      }));
      renderResourceCatalog();
    } catch (error) {
      console.error("Resource catalog load error:", error);
      meshResources = [];
      renderResourceCatalog();
      showResourceActionResult(String(error), "err");
    }
  };

  const updateResourceEditorTypeUI = () => {
    const selected = resourceTypeRadios.find((radio) => radio.checked)?.value ?? "external";
    if (selected === "external") {
      resourceExternalExtra?.classList.remove("hidden");
      resourceInternalExtra?.classList.add("hidden");
    } else {
      resourceExternalExtra?.classList.add("hidden");
      resourceInternalExtra?.classList.remove("hidden");
    }
  };

  const openResourceEditor = (resource: MeshResource | null) => {
    if (!resourceEditorModal) {
      return;
    }
    resourceEditorMode = resource ? "edit" : "create";
    resourceEditorOriginalName = resource?.name ?? null;
    if (resourceEditorTitle) {
      resourceEditorTitle.textContent = resource ? `Edit ${resource.name}` : "Add Mesh Resource";
    }
    if (resourceNameInput) {
      resourceNameInput.value = resource?.name ?? "";
      resourceNameInput.disabled = Boolean(resource);
    }
    if (resourceTargetInput) {
      resourceTargetInput.value = resource?.target ?? "";
    }
    if (resourceAllowedMethodsInput) {
      resourceAllowedMethodsInput.value = (resource?.allowedMethods ?? []).join(", ");
    }
    if (resourceVersionConstraintInput) {
      resourceVersionConstraintInput.value = resource?.versionConstraint ?? "";
    }
    resourceTypeRadios.forEach((radio) => {
      radio.checked = radio.value === (resource?.type ?? "external");
    });
    if (resourceEditorError) {
      resourceEditorError.classList.add("hidden");
      resourceEditorError.textContent = "";
    }
    updateResourceEditorTypeUI();
    resourceEditorModal.classList.remove("hidden");
    gsap.fromTo(
      resourceEditorModal,
      { autoAlpha: 0 },
      { autoAlpha: 1, duration: 0.18, ease: "power2.out" },
    );
  };

  const closeResourceEditor = () => {
    if (!resourceEditorModal) {
      return;
    }
    resourceEditorModal.classList.add("hidden");
    gsap.set(resourceEditorModal, { clearProps: "all" });
  };

  const saveResource = async () => {
    if (!resourceNameInput || !resourceTargetInput || !resourceSaveBtn) {
      return;
    }
    const name = resourceNameInput.value.trim();
    const target = resourceTargetInput.value.trim();
    const type = (resourceTypeRadios.find((radio) => radio.checked)?.value ?? "external") as MeshResourceKind;
    const allowedMethods =
      type === "external"
        ? (resourceAllowedMethodsInput?.value ?? "")
            .split(",")
            .map((value) => value.trim().toUpperCase())
            .filter(Boolean)
        : undefined;
    const versionConstraint =
      type === "internal" ? resourceVersionConstraintInput?.value.trim() || undefined : undefined;

    if (!name) {
      if (resourceEditorError) {
        resourceEditorError.textContent = "Resource name must not be empty.";
        resourceEditorError.classList.remove("hidden");
      }
      return;
    }
    if (!target) {
      if (resourceEditorError) {
        resourceEditorError.textContent = "Target must not be empty.";
        resourceEditorError.classList.remove("hidden");
      }
      return;
    }
    if (type === "external" && !/^https:\/\//i.test(target) && !/\.svc(\.cluster\.local)?(:|$|\/)/i.test(target)) {
      if (resourceEditorError) {
        resourceEditorError.textContent = "External target must be an HTTPS URL or a cluster-local *.svc address.";
        resourceEditorError.classList.remove("hidden");
      }
      return;
    }

    resourceSaveBtn.disabled = true;
    resourceSaveBtn.textContent = "Saving...";
    const previousResources = [...meshResources];
    const payload: MeshResourceInput = { name, type, target, allowedMethods, versionConstraint };
    const optimisticResource: MeshResource = {
      name,
      type,
      target,
      pending: true,
      allowedMethods: allowedMethods ?? [],
      versionConstraint,
    };
    meshResources =
      resourceEditorMode === "edit"
        ? meshResources.map((resource) => (resource.name === name ? optimisticResource : resource))
        : [...meshResources.filter((resource) => resource.name !== name), optimisticResource];
    renderResourceCatalog();
    closeResourceEditor();
    try {
      await invoke("save_resource", { resource: payload });
      showResourceActionResult(
        `Saved ${name} (pending CLI re-seal to promote into integrity.lock).`,
        "ok",
      );
      await loadResources();
    } catch (error) {
      console.error("Resource save error:", error);
      meshResources = previousResources;
      renderResourceCatalog();
      if (resourceEditorError) {
        resourceEditorError.textContent = String(error);
        resourceEditorError.classList.remove("hidden");
      }
    } finally {
      resourceSaveBtn.disabled = false;
      resourceSaveBtn.textContent = "Save";
      resourceEditorOriginalName = null;
    }
  };

  const deleteResource = async (name: string) => {
    if (!confirm(`Delete resource "${name}"? Sealed entries cannot be removed without a CLI re-seal.`)) {
      return;
    }
    const previousResources = [...meshResources];
    meshResources = meshResources.filter((resource) => resource.name !== name);
    renderResourceCatalog();
    try {
      await invoke("delete_resource", { name });
      showResourceActionResult(`Removed ${name} from the workspace overlay.`, "ok");
      await loadResources();
    } catch (error) {
      console.error("Resource delete error:", error);
      meshResources = previousResources;
      renderResourceCatalog();
      showResourceActionResult(String(error), "err");
    }
  };

  const renderAccountMessage = (target: HTMLElement | null, message: string, className: string) => {
    if (!target) {
      return;
    }
    target.textContent = message;
    target.className = className;
    target.classList.remove("hidden");
  };

  const renderIdentityMessage = (message: string, className: string) => {
    renderAccountMessage(identityActionResult, message, className);
  };

  const loadIamUsers = async () => {
    if (!authGatewayValidated) {
      iamUsers = [];
      renderIdentityView();
      return;
    }

    try {
      iamUsers = await invoke<IamUserSummary[]>("iam_list_users");
    } catch (error) {
      console.error("IAM load error:", error);
      iamUsers = [];
      renderIdentityMessage(
        String(error),
        "min-h-24 rounded-xl border border-red-500/30 bg-slate-900 px-4 py-3 font-mono text-xs text-red-400 whitespace-pre-wrap break-words",
      );
    }

    renderIdentityView();
  };

  const renderMeshGraph = (snapshot: MeshGraphSnapshot) => {
    if (meshGraphSource) {
      meshGraphSource.textContent = snapshot.source;
    }
    if (meshGraphStatus) {
      meshGraphStatus.textContent = snapshot.status;
    }
    if (meshRouteCount) {
      meshRouteCount.textContent = String(snapshot.routes.length);
    }
    if (peerCount) {
      peerCount.textContent = String(snapshot.batchTargets.length);
    }
    if (meshRouteList) {
      meshRouteList.innerHTML = "";
      if (snapshot.routes.length === 0) {
        const empty = document.createElement("div");
        empty.className = "rounded-xl border border-slate-800 bg-slate-950/70 p-4 text-sm text-slate-500 md:col-span-2";
        empty.textContent = "No sealed routes were discovered for the current topology snapshot.";
        meshRouteList.appendChild(empty);
      } else {
        snapshot.routes.forEach((route) => {
          const card = document.createElement("div");
          card.className = "rounded-xl border border-slate-800 bg-slate-950/70 p-4";
          card.innerHTML = `
            <div class="flex items-center justify-between gap-4 mb-3">
              <div class="text-sm font-semibold text-white">${route.name}</div>
              <span class="rounded-full border border-slate-700 bg-slate-900 px-2 py-1 text-[11px] uppercase tracking-[0.2em] text-slate-400">${route.role}</span>
            </div>
            <div class="text-xs font-mono text-cyan-300 break-all mb-2">${route.path}</div>
            <div class="text-xs text-slate-500">Targets: ${route.targetCount}</div>
          `;
          meshRouteList.appendChild(card);
        });
      }
    }
    if (meshBatchList) {
      meshBatchList.innerHTML = "";
      if (snapshot.batchTargets.length === 0) {
        const empty = document.createElement("div");
        empty.className = "rounded-xl border border-slate-800 bg-slate-950/70 p-4 text-sm text-slate-500 md:col-span-2";
        empty.textContent = "No batch targets are configured for this manifest.";
        meshBatchList.appendChild(empty);
      } else {
        snapshot.batchTargets.forEach((target) => {
          const card = document.createElement("div");
          card.className = "rounded-xl border border-slate-800 bg-slate-950/70 p-4";
          card.innerHTML = `
            <div class="text-sm font-semibold text-white mb-1">${target}</div>
            <div class="text-xs text-slate-500">Batch workload ready for control-plane dispatch.</div>
          `;
          meshBatchList.appendChild(card);
        });
      }
    }
  };

  const refreshMeshTopology = async () => {
    if (refreshTopologyBtn) {
      refreshTopologyBtn.disabled = true;
      refreshTopologyBtn.textContent = "Refreshing...";
    }

    try {
      const snapshot = await invoke<MeshGraphSnapshot>("get_mesh_graph");
      renderMeshGraph(snapshot);
    } catch (error) {
      console.error("Mesh topology error:", error);
      if (meshGraphStatus) {
        meshGraphStatus.textContent = String(error);
      }
    } finally {
      if (refreshTopologyBtn) {
        refreshTopologyBtn.disabled = false;
        refreshTopologyBtn.textContent = "Refresh Topology";
      }
    }
  };

  const switchView = async (view: ViewName) => {
    const currentPanel = viewPanels[activeView];
    const nextPanel = viewPanels[view];
    if (!nextPanel) {
      return;
    }

    updateNavigationState(view);
    updateHeaderForView(view);

    if (view !== activeView && currentPanel) {
      await gsap.to(currentPanel, {
        autoAlpha: 0,
        y: -12,
        duration: 0.18,
        ease: "power2.inOut",
      });
      currentPanel.classList.add("hidden");
      gsap.set(currentPanel, { clearProps: "all" });
      nextPanel.classList.remove("hidden");
      await gsap.fromTo(
        nextPanel,
        { autoAlpha: 0, y: 18 },
        { autoAlpha: 1, y: 0, duration: 0.24, ease: "power2.out" },
      );
    }

    activeView = view;
    if (view === "topology") {
      void refreshMeshTopology();
    }
    if (view === "identity") {
      renderIdentityView();
    }
    if (view === "account") {
      renderAccountView();
    }
    if (view === "resources") {
      void loadResources();
    }
  };

  const showFirstRunModal = async () => {
    if (!firstRunModal || localStorage.getItem(onboardingStorageKey()) === "complete") {
      return;
    }

    downloadedRecoveryCodes = false;
    recoveryCodes = [];
    clearOnboardingError();
    qrStep?.classList.remove("hidden");
    recoveryCodesStep?.classList.add("hidden");
    if (confirmSavedBtn) {
      confirmSavedBtn.disabled = true;
    }

    firstRunModal.classList.remove("hidden");
    await gsap.fromTo(
      firstRunModal,
      { autoAlpha: 0 },
      { autoAlpha: 1, duration: 0.25, ease: "power2.out" },
    );
  };

  const closeFirstRunModal = async () => {
    if (!firstRunModal) {
      return;
    }

    localStorage.setItem(onboardingStorageKey(), "complete");
    await gsap.to(firstRunModal, {
      autoAlpha: 0,
      duration: 0.25,
      ease: "power2.inOut",
    });
    firstRunModal.classList.add("hidden");
    renderIdentityView();
    renderAccountView();
  };

  const renderRecoveryCodes = (codes: string[]) => {
    if (!codesContainer) {
      return;
    }

    codesContainer.innerHTML = "";
    codes.forEach((code) => {
      const cell = document.createElement("div");
      cell.textContent = code;
      cell.className = "rounded border border-slate-800 bg-slate-900 px-2 py-1";
      codesContainer.appendChild(cell);
    });
  };

  const readIdentityBytes = async (): Promise<number[] | null> => {
    const file = mtlsFile?.files?.[0];
    if (!file) {
      return null;
    }

    const buffer = await file.arrayBuffer();
    return Array.from(new Uint8Array(buffer));
  };

  const hideAuthOverlay = async () => {
    if (!overlay) {
      return;
    }

    await gsap.to(overlay, {
      autoAlpha: 0,
      duration: 0.5,
      pointerEvents: "none",
      ease: "power2.out",
    });
    overlay.classList.add("hidden");
  };

  const connectToNode = async () => {
    if (!nodeUrl || !nodeToken || !connectSubmitBtn || !authUsername) {
      return;
    }

    clearConnectionError();
    connectSubmitBtn.disabled = true;
    connectSubmitBtn.textContent = "Authenticating...";

    try {
      const cert = await readIdentityBytes();
      const response = await invoke<AuthLoginResponse>("authn_login", {
        payload: {
          url: nodeUrl.value,
          username: authUsername.value.trim() || "admin",
          password: nodeToken.value,
          cert,
        },
      });
      const status = await invoke<string>("get_engine_status");

      if (activeFaaS) {
        activeFaaS.innerText = String(status);
      }
      activeOperator = response.username;
      authGatewayValidated = true;
      updateConnectionBadge();
      await loadIamUsers();
      renderAccountView();
      void refreshMeshTopology();
      renderIdentityMessage(
        `Authenticated ${response.username} against ${response.endpoint}. Complete MFA to unlock the dashboard.`,
        "min-h-24 rounded-xl border border-cyan-500/20 bg-slate-900 px-4 py-3 font-mono text-xs text-cyan-300 whitespace-pre-wrap break-words",
      );

      if (response.requiresMfa) {
        await switchAuthStep("mfa");
      } else {
        await hideAuthOverlay();
        await showFirstRunModal();
      }
    } catch (error) {
      authGatewayValidated = false;
      iamUsers = [];
      console.error("Connection error:", error);
      showConnectionError(String(error));
    } finally {
      connectSubmitBtn.disabled = false;
      connectSubmitBtn.textContent = "Authenticate";
    }
  };

  const completeMfa = async () => {
    if (!authMfaCode || !authMfaSubmitBtn) {
      return;
    }

    clearConnectionError();
    const code = authMfaCode.value.replace(/\s+/g, "");
    if (!/^\d{6}$/.test(code)) {
      showConnectionError("Enter a 6-digit MFA code.");
      return;
    }

    authMfaSubmitBtn.disabled = true;
    authMfaSubmitBtn.textContent = "Verifying...";

    try {
      await hideAuthOverlay();
      renderIdentityView();
      renderAccountView();
      await showFirstRunModal();
    } finally {
      authMfaSubmitBtn.disabled = false;
      authMfaSubmitBtn.textContent = "Verify Code";
    }
  };

  const openSignupFlow = async () => {
    resetSignupFlow();
    await switchAuthStep("signup-token");
  };

  const validateSignupToken = async () => {
    if (!nodeUrl || !signupTokenInput || !signupTokenSubmitBtn) {
      return;
    }

    clearConnectionError();
    signupTokenSubmitBtn.disabled = true;
    signupTokenSubmitBtn.textContent = "Validating...";

    try {
      const cert = await readIdentityBytes();
      const claims = await invoke<RegistrationTokenClaims>("validate_signup_token", {
        payload: {
          url: nodeUrl.value,
          token: signupTokenInput.value,
          cert,
        },
      });
      signupClaims = claims;
      recomputeSignupUsername();
      renderSignupClaims(claims);
      await switchAuthStep("signup-profile");
    } catch (error) {
      console.error("Signup token validation error:", error);
      showConnectionError(String(error));
    } finally {
      signupTokenSubmitBtn.disabled = false;
      signupTokenSubmitBtn.textContent = "Validate Invite";
    }
  };

  const stageSignupAccount = async () => {
    if (
      !nodeUrl ||
      !signupTokenInput ||
      !signupFirstName ||
      !signupLastName ||
      !signupUsername ||
      !signupPassword ||
      !signupProfileSubmitBtn
    ) {
      return;
    }

    clearConnectionError();
    signupProfileSubmitBtn.disabled = true;
    signupProfileSubmitBtn.textContent = "Staging...";

    try {
      const cert = await readIdentityBytes();
      const session = await invoke<StagedSignupSession>("stage_signup", {
        payload: {
          url: nodeUrl.value,
          token: signupTokenInput.value,
          firstName: signupFirstName.value,
          lastName: signupLastName.value,
          username: signupUsername.value,
          password: signupPassword.value,
          cert,
        },
      });
      stagedSignup = session;
      await renderStagedSignup(session);
      await switchAuthStep("signup-totp");
    } catch (error) {
      console.error("Signup staging error:", error);
      showConnectionError(String(error));
    } finally {
      signupProfileSubmitBtn.disabled = false;
      signupProfileSubmitBtn.textContent = "Stage Account";
    }
  };

  const finalizeSignup = async () => {
    if (!nodeUrl || !stagedSignup || !signupTotpCode || !signupTotpSubmitBtn) {
      return;
    }

    clearConnectionError();
    const code = signupTotpCode.value.replace(/\s+/g, "");
    if (!/^\d{6}$/.test(code)) {
      showConnectionError("Enter the first 6-digit TOTP code from your authenticator app.");
      return;
    }

    signupTotpSubmitBtn.disabled = true;
    signupTotpSubmitBtn.textContent = "Finalizing...";

    try {
      const cert = await readIdentityBytes();
      const response = await invoke<AuthLoginResponse>("finalize_signup", {
        payload: {
          url: nodeUrl.value,
          sessionId: stagedSignup.sessionId,
          totpCode: code,
          cert,
        },
      });
      const status = await invoke<string>("get_engine_status");

      if (activeFaaS) {
        activeFaaS.innerText = String(status);
      }
      activeOperator = response.username;
      authGatewayValidated = true;
      if (authUsername) {
        authUsername.value = response.username;
      }
      updateConnectionBadge();
      await loadIamUsers();
      renderAccountView();
      renderIdentityMessage(
        `Enrollment completed for ${response.username}. The dashboard is unlocked and recovery codes must now be saved for ${response.endpoint}.`,
        "min-h-24 rounded-xl border border-emerald-500/30 bg-slate-900 px-4 py-3 font-mono text-xs text-emerald-300 whitespace-pre-wrap break-words",
      );
      void refreshMeshTopology();
      await hideAuthOverlay();
      await showFirstRunModal();
    } catch (error) {
      console.error("Signup finalization error:", error);
      showConnectionError(String(error));
    } finally {
      signupTotpSubmitBtn.disabled = false;
      signupTotpSubmitBtn.textContent = "Finalize Enrollment";
    }
  };

  connectSubmitBtn?.addEventListener("click", () => {
    void connectToNode();
  });

  openSignupBtn?.addEventListener("click", () => {
    void openSignupFlow();
  });

  backToLoginBtn?.addEventListener("click", () => {
    void switchAuthStep("login");
  });

  authMfaSubmitBtn?.addEventListener("click", () => {
    void completeMfa();
  });

  signupTokenBackBtn?.addEventListener("click", () => {
    void switchAuthStep("login");
  });

  signupTokenSubmitBtn?.addEventListener("click", () => {
    void validateSignupToken();
  });

  signupProfileBackBtn?.addEventListener("click", () => {
    void switchAuthStep("signup-token");
  });

  signupProfileSubmitBtn?.addEventListener("click", () => {
    void stageSignupAccount();
  });

  signupTotpBackBtn?.addEventListener("click", () => {
    void switchAuthStep("signup-profile");
  });

  signupTotpSubmitBtn?.addEventListener("click", () => {
    void finalizeSignup();
  });

  nodeToken?.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      void connectToNode();
    }
  });

  authMfaCode?.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      void completeMfa();
    }
  });

  signupTokenInput?.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      void validateSignupToken();
    }
  });

  signupPassword?.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      void stageSignupAccount();
    }
  });

  signupConfirmPassword?.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      void stageSignupAccount();
    }
  });

  signupFirstName?.addEventListener("input", () => {
    recomputeSignupUsername();
    refreshSignupValidation();
  });
  signupLastName?.addEventListener("input", () => {
    recomputeSignupUsername();
    refreshSignupValidation();
  });
  signupPassword?.addEventListener("input", () => {
    refreshSignupValidation();
  });
  signupConfirmPassword?.addEventListener("input", () => {
    refreshSignupValidation();
  });

  document.querySelectorAll<HTMLButtonElement>(".password-toggle").forEach((button) => {
    button.addEventListener("click", (event) => {
      event.preventDefault();
      togglePasswordVisibility(button);
    });
  });

  signupTotpCode?.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      void finalizeSignup();
    }
  });

  showRecoveryCodesBtn?.addEventListener("click", async () => {
    clearOnboardingError();
    showRecoveryCodesBtn.disabled = true;
    showRecoveryCodesBtn.textContent = "Generating...";

    try {
      recoveryCodes = await invoke<string[]>("generate_recovery_codes", {
        username: activeOperator,
      });
      renderRecoveryCodes(recoveryCodes);
      qrStep?.classList.add("hidden");
      recoveryCodesStep?.classList.remove("hidden");
    } catch (error) {
      console.error("Recovery code generation error:", error);
      showOnboardingError(String(error));
    } finally {
      showRecoveryCodesBtn.disabled = false;
      showRecoveryCodesBtn.textContent = "Continue to Recovery Codes";
    }
  });

  downloadCodesBtn?.addEventListener("click", () => {
    if (recoveryCodes.length === 0) {
      showOnboardingError("No recovery codes are available to download.");
      return;
    }

    const blob = new Blob([recoveryCodes.join("\n")], { type: "text/plain;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = "tachyon-recovery-codes.txt";
    document.body.appendChild(anchor);
    anchor.click();
    anchor.remove();
    URL.revokeObjectURL(url);

    downloadedRecoveryCodes = true;
    if (confirmSavedBtn) {
      confirmSavedBtn.disabled = false;
    }
  });

  confirmSavedBtn?.addEventListener("click", () => {
    if (!downloadedRecoveryCodes) {
      showOnboardingError("Download the recovery codes before completing onboarding.");
      return;
    }

    void closeFirstRunModal();
  });

  refreshTopologyBtn?.addEventListener("click", () => {
    void refreshMeshTopology();
  });

  identityMfaBtn?.addEventListener("click", () => {
    void showFirstRunModal();
  });

  newUserBtn?.addEventListener("click", () => {
    renderIdentityMessage(
      "Remote user provisioning is not exposed by the current admin API. Use PAT scopes and recovery workflows from the authenticated session.",
      "min-h-24 rounded-xl border border-amber-500/30 bg-slate-900 px-4 py-3 font-mono text-xs text-amber-300 whitespace-pre-wrap break-words",
    );
  });

  iamUserList?.addEventListener("click", async (event) => {
    const target = event.target as HTMLElement | null;
    if (!target?.dataset.action) {
      return;
    }

    if (target.dataset.action === "roles-hint") {
      renderIdentityMessage(
        "RBAC is currently derived from JWT roles and PAT scopes validated by AuthN/AuthZ on `/admin/*` routes.",
        "min-h-24 rounded-xl border border-slate-700 bg-slate-900 px-4 py-3 font-mono text-xs text-slate-300 whitespace-pre-wrap break-words",
      );
      return;
    }

    if (target.dataset.action !== "regen-mfa") {
      return;
    }

    const username = target.dataset.username ?? "";
    target.setAttribute("disabled", "true");
    renderIdentityMessage(
      `Rotating recovery bundle for ${username}...`,
      "min-h-24 rounded-xl border border-slate-700 bg-slate-900 px-4 py-3 font-mono text-xs text-slate-300 whitespace-pre-wrap break-words",
    );

    try {
      const codes = await invoke<string[]>("iam_regen_mfa", { username });
      renderIdentityMessage(
        codes.join("\n"),
        "min-h-24 rounded-xl border border-emerald-500/30 bg-slate-900 px-4 py-3 font-mono text-xs text-emerald-300 whitespace-pre-wrap break-words",
      );
      renderIdentityView();
    } catch (error) {
      console.error("IAM recovery rotation error:", error);
      renderIdentityMessage(
        String(error),
        "min-h-24 rounded-xl border border-red-500/30 bg-slate-900 px-4 py-3 font-mono text-xs text-red-400 whitespace-pre-wrap break-words",
      );
    } finally {
      target.removeAttribute("disabled");
    }
  });

  patGenerateBtn?.addEventListener("click", async () => {
    const name = patNameInput?.value.trim() ?? "";
    const scopes = (patScopesInput?.value ?? "")
      .split(",")
      .map((scope) => scope.trim())
      .filter(Boolean);
    const ttlDays = Number(patTtlInput?.value ?? "30");

    if (!name) {
      renderAccountMessage(
        patResult,
        "PAT name must not be empty.",
        "mt-4 p-3 bg-slate-900 border border-red-500/30 text-red-400 font-mono text-xs break-all rounded-lg",
      );
      return;
    }
    if (scopes.length === 0) {
      renderAccountMessage(
        patResult,
        "Provide at least one PAT scope.",
        "mt-4 p-3 bg-slate-900 border border-red-500/30 text-red-400 font-mono text-xs break-all rounded-lg",
      );
      return;
    }

    patGenerateBtn.disabled = true;
    patGenerateBtn.textContent = "Generating...";
    renderAccountMessage(
      patResult,
      "Issuing scoped PAT...",
      "mt-4 p-3 bg-slate-900 border border-slate-700 text-slate-300 font-mono text-xs break-all rounded-lg",
    );

    try {
      const token = await invoke<string>("generate_pat", {
        name,
        scopes,
        ttlDays,
      });
      renderAccountMessage(
        patResult,
        token,
        "mt-4 p-3 bg-slate-900 border border-emerald-500/30 text-emerald-400 font-mono text-xs break-all rounded-lg",
      );
    } catch (error) {
      console.error("PAT generation error:", error);
      renderAccountMessage(
        patResult,
        String(error),
        "mt-4 p-3 bg-slate-900 border border-red-500/30 text-red-400 font-mono text-xs break-all rounded-lg",
      );
    } finally {
      patGenerateBtn.disabled = false;
      patGenerateBtn.textContent = "Generate Token";
    }
  });

  regen2faBtn?.addEventListener("click", async () => {
    regen2faBtn.disabled = true;
    regen2faBtn.textContent = "Regenerating...";
    renderAccountMessage(
      accountSecurityResult,
      "Rotating recovery bundle...",
      "mt-4 p-3 bg-slate-900 border border-slate-700 text-slate-300 font-mono text-xs whitespace-pre-wrap break-words rounded-lg",
    );

    try {
      const codes = await invoke<string[]>("regenerate_account_security");
      renderAccountMessage(
        accountSecurityResult,
        codes.join("\n"),
        "mt-4 p-3 bg-slate-900 border border-emerald-500/30 text-emerald-300 font-mono text-xs whitespace-pre-wrap break-words rounded-lg",
      );
    } catch (error) {
      console.error("Account security regeneration error:", error);
      renderAccountMessage(
        accountSecurityResult,
        String(error),
        "mt-4 p-3 bg-slate-900 border border-red-500/30 text-red-400 font-mono text-xs whitespace-pre-wrap break-words rounded-lg",
      );
    } finally {
      regen2faBtn.disabled = false;
      regen2faBtn.textContent = "Regenerate Recovery Bundle";
    }
  });

  navLinks.forEach((link) => {
    link.addEventListener("click", (event) => {
      event.preventDefault();
      const view = link.dataset.view as ViewName | undefined;
      if (!view) {
        return;
      }

      void switchView(view);
    });
  });

  assetUploadBtn?.addEventListener("click", async () => {
    const file = assetUploadInput?.files?.[0];
    if (!file) {
      if (assetUploadResult) {
        assetUploadResult.textContent = "Select a .wasm asset first.";
      }
      return;
    }

    assetUploadBtn.disabled = true;
    assetUploadBtn.textContent = "Uploading...";
    if (assetUploadResult) {
      assetUploadResult.textContent = "Uploading asset to the embedded registry...";
    }

    try {
      const buffer = await file.arrayBuffer();
      const assetUri = await invoke<string>("push_asset", {
        path: file.name,
        bytes: Array.from(new Uint8Array(buffer)),
      });

      if (assetUploadResult) {
        assetUploadResult.textContent = assetUri;
      }
    } catch (error) {
      console.error("Asset upload error:", error);
      if (assetUploadResult) {
        assetUploadResult.textContent = String(error);
      }
    } finally {
      assetUploadBtn.disabled = false;
      assetUploadBtn.textContent = "Push Asset to Mesh";
    }
  });

  void listen<number>("upload_progress", (event) => {
    if (!modelProgress) {
      return;
    }

    const percentage = Math.max(0, Math.min(100, Number(event.payload) || 0));
    gsap.to(modelProgress, {
      width: `${percentage}%`,
      duration: 0.2,
      ease: "power1.out",
    });
  });

  modelUploadBtn?.addEventListener("click", async () => {
    const file = modelUploadInput?.files?.[0] as (File & { path?: string }) | undefined;
    if (!file) {
      if (modelUploadResult) {
        modelUploadResult.textContent = "Select a model file first.";
      }
      return;
    }

    if (!file.path) {
      if (modelUploadResult) {
        modelUploadResult.textContent = "This runtime did not expose a native file path for the selected model.";
      }
      return;
    }

    modelUploadBtn.disabled = true;
    modelUploadBtn.textContent = "Streaming...";
    if (modelUploadResult) {
      modelUploadResult.textContent = "Initializing multipart upload...";
    }
    if (modelProgress) {
      gsap.set(modelProgress, { width: "0%" });
    }

    try {
      const modelPath = await invoke<string>("push_large_model", {
        path: file.path,
      });
      if (modelUploadResult) {
        modelUploadResult.textContent = modelPath;
      }
    } catch (error) {
      console.error("Model upload error:", error);
      if (modelUploadResult) {
        modelUploadResult.textContent = String(error);
      }
    } finally {
      modelUploadBtn.disabled = false;
      modelUploadBtn.textContent = "Stream Model to Disk";
    }
  });

  refreshResourcesBtn?.addEventListener("click", () => {
    void loadResources();
  });

  addResourceBtn?.addEventListener("click", () => {
    openResourceEditor(null);
  });

  resourceCancelBtn?.addEventListener("click", () => {
    closeResourceEditor();
  });

  resourceSaveBtn?.addEventListener("click", () => {
    void saveResource();
  });

  resourceTypeRadios.forEach((radio) => {
    radio.addEventListener("change", updateResourceEditorTypeUI);
  });

  resourceSearchInput?.addEventListener("input", () => {
    renderResourceCatalog();
  });

  resourceFilterButtons.forEach((button) => {
    button.addEventListener("click", () => {
      const value = (button.dataset.resourceFilter ?? "all") as "all" | MeshResourceKind;
      resourceFilter = value;
      resourceFilterButtons.forEach((other) => {
        const active = other === button;
        other.classList.toggle("border-cyan-500/40", active);
        other.classList.toggle("bg-cyan-500/10", active);
        other.classList.toggle("text-cyan-300", active);
        other.classList.toggle("border-slate-700", !active);
        other.classList.toggle("text-slate-400", !active);
      });
      renderResourceCatalog();
    });
  });

  resourceTableBody?.addEventListener("click", (event) => {
    const target = event.target as HTMLElement | null;
    const action = target?.dataset.action;
    const name = target?.dataset.name;
    if (!action || !name) {
      return;
    }
    if (action === "resource-edit") {
      const resource = meshResources.find((entry) => entry.name === name) ?? null;
      openResourceEditor(resource);
    } else if (action === "resource-delete") {
      void deleteResource(name);
    }
  });

  refreshBtn?.addEventListener("click", async () => {
    gsap.fromTo(refreshBtn, { scale: 0.95 }, { scale: 1, duration: 0.2, ease: "bounce.out" });

    try {
      const response = await invoke<string>("get_engine_status");

      if (activeFaaS) {
        activeFaaS.innerText = String(response);
        gsap.fromTo(activeFaaS, { color: "#22d3ee" }, { color: "#ffffff", duration: 1 });
      }
      if (activeView === "topology") {
        void refreshMeshTopology();
      }
      renderIdentityView();
    } catch (error) {
      console.error("Tauri invoke error:", error);
      if (activeFaaS) {
        activeFaaS.innerText = "Err";
      }
    }
  });

  nodeUrl?.addEventListener("input", () => {
    updateConnectionBadge();
    renderIdentityView();
    renderAccountView();
  });

  authUsername?.addEventListener("input", () => {
    activeOperator = authUsername.value.trim() || "admin";
    renderIdentityView();
  });

  nodeToken?.addEventListener("input", () => {
    renderIdentityView();
    renderAccountView();
  });

  updateConnectionBadge();
  updateHeaderForView(activeView);
  updateNavigationState(activeView);
  resetSignupFlow();
  renderIdentityView();
  renderAccountView();
  void refreshMeshTopology();
  console.info("[tachyon-ui] navigation routing enabled", {
    viewCount: viewContainers.length,
    dashboard: "view-dashboard",
    topology: "view-topology",
    registry: "view-registry",
    identity: "view-identity",
    account: "view-account",
    resources: "view-resources",
    broker: "view-broker",
  });
});

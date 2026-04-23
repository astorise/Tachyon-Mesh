import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import gsap from "gsap";

const configuredNodeUrl = (import.meta.env.VITE_TACHYON_NODE_URL ?? "").trim();
const configuredNodeToken = (import.meta.env.VITE_TACHYON_NODE_TOKEN ?? "").trim();

type ViewName = "dashboard" | "topology" | "registry" | "identity" | "account" | "broker";

type AuthLoginResponse = {
  username: string;
  endpoint: string;
  requiresMfa: boolean;
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
  const overlay = document.getElementById("auth-overlay");
  const firstRunModal = document.getElementById("first-run-modal");
  const authLoginStep = document.getElementById("auth-step-login");
  const authMfaStep = document.getElementById("auth-step-mfa");
  const nodeUrl = document.getElementById("auth-url") as HTMLInputElement | null;
  const authUsername = document.getElementById("auth-username") as HTMLInputElement | null;
  const nodeToken = document.getElementById("auth-password") as HTMLInputElement | null;
  const mtlsFile = document.getElementById("auth-mtls") as HTMLInputElement | null;
  const connectSubmitBtn = document.getElementById("btn-login-submit") as HTMLButtonElement | null;
  const authMfaCode = document.getElementById("auth-mfa-code") as HTMLInputElement | null;
  const authMfaSubmitBtn = document.getElementById("btn-mfa-submit") as HTMLButtonElement | null;
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

  let downloadedRecoveryCodes = false;
  let recoveryCodes: string[] = [];
  let activeView: ViewName = "dashboard";
  let activeOperator = "admin";
  let authGatewayValidated = false;
  let iamUsers: IamUserSummary[] = [];

  const viewPanels: Record<ViewName, HTMLElement | null> = {
    dashboard: document.getElementById("view-dashboard"),
    topology: document.getElementById("view-topology"),
    registry: document.getElementById("view-registry"),
    identity: document.getElementById("view-identity"),
    account: document.getElementById("view-account"),
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

      if (authLoginStep && authMfaStep) {
        await gsap.to(authLoginStep, {
          autoAlpha: 0,
          y: -12,
          duration: 0.2,
          ease: "power2.inOut",
        });
        authLoginStep.classList.add("hidden");
        authMfaStep.classList.remove("hidden");
        await gsap.fromTo(
          authMfaStep,
          { autoAlpha: 0, y: 18 },
          { autoAlpha: 1, y: 0, duration: 0.24, ease: "power2.out" },
        );
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
    if (!authMfaCode || !authMfaSubmitBtn || !overlay) {
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
      await gsap.to(overlay, {
        autoAlpha: 0,
        duration: 0.5,
        pointerEvents: "none",
        ease: "power2.out",
      });
      overlay.classList.add("hidden");
      renderIdentityView();
      renderAccountView();
      await showFirstRunModal();
    } finally {
      authMfaSubmitBtn.disabled = false;
      authMfaSubmitBtn.textContent = "Verify Code";
    }
  };

  connectSubmitBtn?.addEventListener("click", () => {
    void connectToNode();
  });

  authMfaSubmitBtn?.addEventListener("click", () => {
    void completeMfa();
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

  showRecoveryCodesBtn?.addEventListener("click", async () => {
    clearOnboardingError();
    showRecoveryCodesBtn.disabled = true;
    showRecoveryCodesBtn.textContent = "Generating...";

    try {
      recoveryCodes = await invoke<string[]>("generate_recovery_codes", {
        username: "admin",
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
    broker: "view-broker",
  });
});

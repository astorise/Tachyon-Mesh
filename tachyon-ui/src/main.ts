import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import gsap from "gsap";

type ViewName = "dashboard" | "topology" | "deployments" | "identity" | "ai-broker";

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
  const overlay = document.getElementById("connection-overlay");
  const firstRunModal = document.getElementById("first-run-modal");
  const nodeUrl = document.getElementById("node-url") as HTMLInputElement | null;
  const nodeToken = document.getElementById("node-token") as HTMLInputElement | null;
  const mtlsFile = document.getElementById("mtls-file") as HTMLInputElement | null;
  const connectSubmitBtn = document.getElementById("connect-btn") as HTMLButtonElement | null;
  const connectionError = document.getElementById("conn-error");
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
  const navLinks = Array.from(document.querySelectorAll<HTMLAnchorElement>(".nav-link[data-view]"));
  const refreshTopologyBtn = document.getElementById("refresh-topology-btn") as HTMLButtonElement | null;
  const meshGraphSource = document.getElementById("mesh-graph-source");
  const meshGraphStatus = document.getElementById("mesh-graph-status");
  const meshRouteCount = document.getElementById("mesh-route-count");
  const meshRouteList = document.getElementById("mesh-route-list");
  const meshBatchList = document.getElementById("mesh-batch-list");
  const identityUserRows = document.getElementById("identity-user-rows");
  const identityConnectionSource = document.getElementById("identity-connection-source");
  const identityMfaStatus = document.getElementById("identity-mfa-status");
  const identityRecoveryStatus = document.getElementById("identity-recovery-status");
  const identityMfaBtn = document.getElementById("identity-mfa-btn") as HTMLButtonElement | null;

  let downloadedRecoveryCodes = false;
  let recoveryCodes: string[] = [];
  let activeView: ViewName = "dashboard";

  const viewPanels: Record<ViewName, HTMLElement | null> = {
    dashboard: document.getElementById("view-dashboard"),
    topology: document.getElementById("view-topology"),
    deployments: document.getElementById("view-deployments"),
    identity: document.getElementById("view-identity"),
    "ai-broker": document.getElementById("view-ai-broker"),
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
    deployments: {
      title: "FaaS Deployments",
      subtitle: "Publish WebAssembly assets directly into the embedded mesh registry.",
    },
    identity: {
      title: "Identity",
      subtitle: "Review the administrative principal and security onboarding posture.",
    },
    "ai-broker": {
      title: "AI Model Broker",
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

    connectionError.textContent = "Connection failed.";
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
    const connected = Boolean(nodeUrl?.value.trim() && nodeToken?.value.trim());
    const recoveryReady = localStorage.getItem(onboardingStorageKey()) === "complete";

    if (identityConnectionSource) {
      identityConnectionSource.textContent = endpoint;
    }
    if (identityMfaStatus) {
      identityMfaStatus.textContent = recoveryReady ? "Recovery bundle secured" : "Onboarding required";
    }
    if (identityRecoveryStatus) {
      identityRecoveryStatus.textContent = recoveryReady
        ? "Downloaded and acknowledged for this endpoint"
        : "Pending download acknowledgement";
    }
    if (identityMfaBtn) {
      identityMfaBtn.disabled = recoveryReady;
      identityMfaBtn.textContent = recoveryReady ? "Recovery Bundle Secured" : "Continue Security Setup";
      identityMfaBtn.classList.toggle("opacity-60", recoveryReady);
      identityMfaBtn.classList.toggle("cursor-not-allowed", recoveryReady);
    }
    if (identityUserRows) {
      const rowState = connected ? "Connected" : "Awaiting connection";
      const securityState = recoveryReady ? "Protected" : "Needs recovery setup";
      identityUserRows.innerHTML = `
        <tr class="border-t border-slate-800">
          <td class="py-3 text-white">admin</td>
          <td class="py-3 text-slate-400">admin, ops</td>
          <td class="py-3 text-slate-300">${rowState} / ${securityState}</td>
        </tr>
      `;
    }
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
    if (!nodeUrl || !nodeToken || !connectSubmitBtn) {
      return;
    }

    clearConnectionError();
    connectSubmitBtn.disabled = true;
    connectSubmitBtn.textContent = "Connecting...";

    try {
      const cert = await readIdentityBytes();
      const response = await invoke<string>("connect_to_node", {
        url: nodeUrl.value,
        token: nodeToken.value,
        cert,
      });

      if (activeFaaS) {
        activeFaaS.innerText = String(response);
      }
      updateConnectionBadge();
      renderIdentityView();
      void refreshMeshTopology();

      if (overlay) {
        await gsap.to(overlay, {
          autoAlpha: 0,
          duration: 0.35,
          ease: "power2.out",
        });
        overlay.classList.add("hidden");
      }

      await showFirstRunModal();
    } catch (error) {
      console.error("Connection error:", error);
      showConnectionError(String(error));
    } finally {
      connectSubmitBtn.disabled = false;
      connectSubmitBtn.textContent = "Establish Connection";
    }
  };

  connectSubmitBtn?.addEventListener("click", () => {
    void connectToNode();
  });

  nodeToken?.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      void connectToNode();
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
  });

  nodeToken?.addEventListener("input", () => {
    renderIdentityView();
  });

  updateConnectionBadge();
  updateHeaderForView(activeView);
  updateNavigationState(activeView);
  renderIdentityView();
  void refreshMeshTopology();
  console.info("[tachyon-ui] navigation routing enabled", {
    dashboard: "view-dashboard",
    topology: "view-topology",
    deployments: "view-deployments",
    identity: "view-identity",
    aiBroker: "view-ai-broker",
  });
});

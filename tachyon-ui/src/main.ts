import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import gsap from 'gsap';

document.addEventListener('DOMContentLoaded', () => {
  const refreshBtn = document.getElementById('refresh-btn');
  const activeFaaS = document.getElementById('active-faas');
  const overlay = document.getElementById('connection-overlay');
  const firstRunModal = document.getElementById('first-run-modal');
  const nodeUrl = document.getElementById('node-url') as HTMLInputElement | null;
  const nodeToken = document.getElementById('node-token') as HTMLInputElement | null;
  const mtlsFile = document.getElementById('mtls-file') as HTMLInputElement | null;
  const connectSubmitBtn = document.getElementById('connect-btn') as HTMLButtonElement | null;
  const connectionError = document.getElementById('conn-error');
  const qrStep = document.getElementById('qr-step');
  const recoveryCodesStep = document.getElementById('recovery-codes-step');
  const showRecoveryCodesBtn = document.getElementById('show-recovery-codes-btn') as HTMLButtonElement | null;
  const codesContainer = document.getElementById('codes-container');
  const downloadCodesBtn = document.getElementById('download-codes-btn') as HTMLButtonElement | null;
  const confirmSavedBtn = document.getElementById('confirm-saved-btn') as HTMLButtonElement | null;
  const onboardingError = document.getElementById('onboarding-error');
  const assetUploadInput = document.getElementById('asset-upload') as HTMLInputElement | null;
  const assetUploadBtn = document.getElementById('asset-upload-btn') as HTMLButtonElement | null;
  const assetUploadResult = document.getElementById('asset-upload-result');
  const modelUploadInput = document.getElementById('model-upload') as HTMLInputElement | null;
  const modelUploadBtn = document.getElementById('model-upload-btn') as HTMLButtonElement | null;
  const modelUploadResult = document.getElementById('model-upload-result');
  const modelProgress = document.getElementById('model-progress');
  let downloadedRecoveryCodes = false;
  let recoveryCodes: string[] = [];

  // 1. Initial GSAP Entrance Animations
  const tl = gsap.timeline();

  tl.from('.sidebar', { x: -50, opacity: 0, duration: 0.6, ease: 'power3.out' })
    .from('.header', { y: -20, opacity: 0, duration: 0.4, ease: 'power2.out' }, '-=0.4')
    .from('.stagger-card', { 
      y: 30, 
      opacity: 0, 
      duration: 0.6, 
      stagger: 0.1, 
      ease: 'back.out(1.2)' 
    }, '-=0.2');

  // Continual subtle pulse for the status dot
  gsap.to('.pulse-dot', {
    opacity: 0.4,
    scale: 0.8,
    duration: 1.5,
    repeat: -1,
    yoyo: true,
    ease: 'sine.inOut'
  });

  const showConnectionError = (message: string) => {
    if (!connectionError) {
      return;
    }

    connectionError.textContent = message;
    connectionError.classList.remove('hidden');
  };

  const clearConnectionError = () => {
    if (!connectionError) {
      return;
    }

    connectionError.textContent = 'Connection failed.';
    connectionError.classList.add('hidden');
  };

  const showOnboardingError = (message: string) => {
    if (!onboardingError) {
      return;
    }

    onboardingError.textContent = message;
    onboardingError.classList.remove('hidden');
  };

  const clearOnboardingError = () => {
    if (!onboardingError) {
      return;
    }

    onboardingError.textContent = 'Unable to complete security onboarding.';
    onboardingError.classList.add('hidden');
  };

  const onboardingStorageKey = () => {
    const url = nodeUrl?.value.trim() || 'default';
    return `tachyon:onboarding:${url}`;
  };

  const showFirstRunModal = async () => {
    if (!firstRunModal || localStorage.getItem(onboardingStorageKey()) === 'complete') {
      return;
    }

    downloadedRecoveryCodes = false;
    recoveryCodes = [];
    clearOnboardingError();
    qrStep?.classList.remove('hidden');
    recoveryCodesStep?.classList.add('hidden');
    if (confirmSavedBtn) {
      confirmSavedBtn.disabled = true;
    }

    firstRunModal.classList.remove('hidden');
    await gsap.fromTo(
      firstRunModal,
      { autoAlpha: 0 },
      { autoAlpha: 1, duration: 0.25, ease: 'power2.out' }
    );
  };

  const closeFirstRunModal = async () => {
    if (!firstRunModal) {
      return;
    }

    localStorage.setItem(onboardingStorageKey(), 'complete');
    await gsap.to(firstRunModal, {
      autoAlpha: 0,
      duration: 0.25,
      ease: 'power2.inOut',
    });
    firstRunModal.classList.add('hidden');
  };

  const renderRecoveryCodes = (codes: string[]) => {
    if (!codesContainer) {
      return;
    }

    codesContainer.innerHTML = '';
    codes.forEach((code) => {
      const cell = document.createElement('div');
      cell.textContent = code;
      cell.className = 'rounded border border-slate-800 bg-slate-900 px-2 py-1';
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
    connectSubmitBtn.textContent = 'Connecting...';

    try {
      const cert = await readIdentityBytes();
      const response = await invoke<string>('connect_to_node', {
        url: nodeUrl.value,
        token: nodeToken.value,
        cert,
      });

      if (activeFaaS) {
        activeFaaS.innerText = String(response);
      }

      if (overlay) {
        await gsap.to(overlay, {
          autoAlpha: 0,
          duration: 0.35,
          ease: 'power2.out',
        });
        overlay.classList.add('hidden');
      }

      await showFirstRunModal();
    } catch (error) {
      console.error('Connection error:', error);
      showConnectionError(String(error));
    } finally {
      connectSubmitBtn.disabled = false;
      connectSubmitBtn.textContent = 'Établir la connexion';
    }
  };

  connectSubmitBtn?.addEventListener('click', () => {
    void connectToNode();
  });

  nodeToken?.addEventListener('keydown', (event) => {
    if (event.key === 'Enter') {
      event.preventDefault();
      void connectToNode();
    }
  });

  showRecoveryCodesBtn?.addEventListener('click', async () => {
    clearOnboardingError();
    showRecoveryCodesBtn.disabled = true;
    showRecoveryCodesBtn.textContent = 'Generating...';

    try {
      recoveryCodes = await invoke<string[]>('generate_recovery_codes', {
        username: 'admin',
      });
      renderRecoveryCodes(recoveryCodes);
      qrStep?.classList.add('hidden');
      recoveryCodesStep?.classList.remove('hidden');
    } catch (error) {
      console.error('Recovery code generation error:', error);
      showOnboardingError(String(error));
    } finally {
      showRecoveryCodesBtn.disabled = false;
      showRecoveryCodesBtn.textContent = 'Continue to Recovery Codes';
    }
  });

  downloadCodesBtn?.addEventListener('click', () => {
    if (recoveryCodes.length === 0) {
      showOnboardingError('No recovery codes are available to download.');
      return;
    }

    const blob = new Blob([recoveryCodes.join('\n')], { type: 'text/plain;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement('a');
    anchor.href = url;
    anchor.download = 'tachyon-recovery-codes.txt';
    document.body.appendChild(anchor);
    anchor.click();
    anchor.remove();
    URL.revokeObjectURL(url);

    downloadedRecoveryCodes = true;
    if (confirmSavedBtn) {
      confirmSavedBtn.disabled = false;
    }
  });

  confirmSavedBtn?.addEventListener('click', () => {
    if (!downloadedRecoveryCodes) {
      showOnboardingError('Download the recovery codes before completing onboarding.');
      return;
    }

    void closeFirstRunModal();
  });

  assetUploadBtn?.addEventListener('click', async () => {
    const file = assetUploadInput?.files?.[0];
    if (!file) {
      if (assetUploadResult) {
        assetUploadResult.textContent = 'Select a .wasm asset first.';
      }
      return;
    }

    assetUploadBtn.disabled = true;
    assetUploadBtn.textContent = 'Uploading...';
    if (assetUploadResult) {
      assetUploadResult.textContent = 'Uploading asset to the embedded registry...';
    }

    try {
      const buffer = await file.arrayBuffer();
      const assetUri = await invoke<string>('push_asset', {
        path: file.name,
        bytes: Array.from(new Uint8Array(buffer)),
      });

      if (assetUploadResult) {
        assetUploadResult.textContent = assetUri;
      }
    } catch (error) {
      console.error('Asset upload error:', error);
      if (assetUploadResult) {
        assetUploadResult.textContent = String(error);
      }
    } finally {
      assetUploadBtn.disabled = false;
      assetUploadBtn.textContent = 'Push Asset to Mesh';
    }
  });

  void listen<number>('upload_progress', (event) => {
    if (!modelProgress) {
      return;
    }

    const percentage = Math.max(0, Math.min(100, Number(event.payload) || 0));
    gsap.to(modelProgress, {
      width: `${percentage}%`,
      duration: 0.2,
      ease: 'power1.out',
    });
  });

  modelUploadBtn?.addEventListener('click', async () => {
    const file = modelUploadInput?.files?.[0] as (File & { path?: string }) | undefined;
    if (!file) {
      if (modelUploadResult) {
        modelUploadResult.textContent = 'Select a model file first.';
      }
      return;
    }

    if (!file.path) {
      if (modelUploadResult) {
        modelUploadResult.textContent = 'This runtime did not expose a native file path for the selected model.';
      }
      return;
    }

    modelUploadBtn.disabled = true;
    modelUploadBtn.textContent = 'Streaming...';
    if (modelUploadResult) {
      modelUploadResult.textContent = 'Initializing multipart upload...';
    }
    if (modelProgress) {
      gsap.set(modelProgress, { width: '0%' });
    }

    try {
      const modelPath = await invoke<string>('push_large_model', {
        path: file.path,
      });
      if (modelUploadResult) {
        modelUploadResult.textContent = modelPath;
      }
    } catch (error) {
      console.error('Model upload error:', error);
      if (modelUploadResult) {
        modelUploadResult.textContent = String(error);
      }
    } finally {
      modelUploadBtn.disabled = false;
      modelUploadBtn.textContent = 'Stream Model to Disk';
    }
  });

  // 2. Tauri Bridge Logic
  refreshBtn?.addEventListener('click', async () => {
    // Animate button click
    gsap.fromTo(refreshBtn, { scale: 0.95 }, { scale: 1, duration: 0.2, ease: 'bounce.out' });

    try {
      // NOTE FOR CODEX: Keep get_engine_status wired through the shared tachyon-client layer.
      const response = await invoke('get_engine_status');
      
      if (activeFaaS) {
        activeFaaS.innerText = String(response);
        // Flash effect on data update
        gsap.fromTo(activeFaaS, { color: '#22d3ee' }, { color: '#ffffff', duration: 1 });
      }
    } catch (error) {
      console.error('Tauri invoke error:', error);
      if (activeFaaS) activeFaaS.innerText = "Err";
    }
  });
});

import { invoke as tauriInvoke } from "@tauri-apps/api/core";

import { connectionStore } from "../stores/connectionStore";
import { ensureMfa } from "./authSudo";
import { translateBackendError } from "./i18n";

export const reconnectDelayMs = (retryCount: number): number => Math.min(1000 * 2 ** retryCount, 30000);

const sleep = (delayMs: number) => new Promise((resolve) => window.setTimeout(resolve, delayMs));
let reconnectLoop: Promise<void> | null = null;

type ApplyConfigurationResponse = {
  success: boolean;
  message: string;
  staged: boolean;
  requiresSeal: boolean;
};

type SealApplyOutcome = {
  success: boolean;
  message: string;
  configVersion: number;
};

export async function resilientInvoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    if (requiresStepUp(command)) {
      await ensureMfa();
    }
    const result = await tauriInvoke<T>(command, args);
    if (command === "apply_configuration" && isStagedConfiguration(result)) {
      window.dispatchEvent(new CustomEvent("config:staged", { detail: result }));
    }
    connectionStore.getState().resetRetry();
    connectionStore.getState().setStatus("connected");
    return result;
  } catch (error) {
    connectionStore.getState().setStatus("disconnected");
    startReconnectLoop();
    const raw = error instanceof Error ? error.message : String(error);
    throw new Error(translateBackendError(raw));
  }
}

export async function applyAndSeal(domain: string, payload: unknown): Promise<SealApplyOutcome | ApplyConfigurationResponse> {
  try {
    const dryRun = await tauriInvoke<ApplyConfigurationResponse>("apply_configuration", {
      domain,
      payload,
      dryRun: true,
    });
    if (!dryRun.success) {
      return dryRun;
    }
    const confirmed = confirmApplyDiff(domain, payload, dryRun.message);
    if (!confirmed) {
      return {
        success: false,
        message: "Configuration apply cancelled.",
        staged: false,
        requiresSeal: false,
      };
    }

    await ensureMfa();
    const staged = await tauriInvoke<ApplyConfigurationResponse>("apply_configuration", {
      domain,
      payload,
      dryRun: false,
    });
    if (!staged.success) {
      return staged;
    }
    const sealed = await tauriInvoke<SealApplyOutcome>("seal_and_apply_manifest");
    connectionStore.getState().resetRetry();
    connectionStore.getState().setStatus("connected");
    window.dispatchEvent(new CustomEvent("config:applied", { detail: { domain, staged, sealed } }));
    return sealed;
  } catch (error) {
    connectionStore.getState().setStatus("disconnected");
    startReconnectLoop();
    const raw = error instanceof Error ? error.message : String(error);
    throw new Error(translateBackendError(raw));
  }
}

function requiresStepUp(command: string): boolean {
  return new Set([
    "seal_and_apply_manifest",
    "save_resource",
    "delete_resource",
    "push_asset",
    "push_large_model",
    "generate_operator_invite",
    "generate_pat",
    "iam_regen_mfa",
    "generate_recovery_codes",
    "regenerate_account_security",
  ]).has(command);
}

function confirmApplyDiff(domain: string, payload: unknown, message: string): boolean {
  const preview = JSON.stringify(payload, null, 2) ?? "";
  const diff = preview.length > 2400 ? `${preview.slice(0, 2400)}\n...` : preview;
  if (typeof window.confirm !== "function") {
    return true;
  }
  return window.confirm(`${message}\n\n${domain}\n${diff}`);
}

function isStagedConfiguration(result: unknown): result is { requiresSeal: boolean; staged: boolean } {
  return (
    typeof result === "object" &&
    result !== null &&
    (result as { requiresSeal?: unknown }).requiresSeal === true &&
    (result as { staged?: unknown }).staged === true
  );
}

function startReconnectLoop(): void {
  if (reconnectLoop) {
    return;
  }
  reconnectLoop = (async () => {
    while (connectionStore.getState().status !== "connected") {
      const retryCount = connectionStore.getState().retryCount;
      connectionStore.getState().setStatus("reconnecting");
      console.info(`tachyon-ui reconnect attempt ${retryCount + 1}; waiting ${reconnectDelayMs(retryCount)}ms`);
      await sleep(reconnectDelayMs(retryCount));
      try {
        await tauriInvoke("get_engine_status");
        connectionStore.getState().resetRetry();
        connectionStore.getState().setStatus("connected");
      } catch {
        connectionStore.getState().incrementRetry();
        connectionStore.getState().setStatus("disconnected");
      }
    }
    reconnectLoop = null;
  })();
}

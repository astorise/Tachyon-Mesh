import { invoke as rawInvoke } from "@tauri-apps/api/core";

export type UiLogLevel = "trace" | "debug" | "info" | "warn" | "error";

const MAX_MESSAGE_CHARS = 8192;
let globalHandlersInstalled = false;

export function logUiEvent(level: UiLogLevel, source: string, message: string): void {
  const payload = {
    level,
    source,
    message: message.slice(0, MAX_MESSAGE_CHARS),
  };
  void rawInvoke<void>("log_frontend_event", { payload }).catch(() => undefined);
}

export function logUiError(source: string, error: unknown): void {
  logUiEvent("error", source, errorMessage(error));
}

export async function loggedInvoke<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  try {
    return await rawInvoke<T>(command, args);
  } catch (error) {
    logUiError(`ipc.${command}`, error);
    throw error;
  }
}

export function installGlobalErrorLogging(): void {
  if (globalHandlersInstalled) {
    return;
  }
  globalHandlersInstalled = true;
  window.addEventListener("error", (event) => {
    logUiEvent("error", "window.error", event.message || errorMessage(event.error));
  });
  window.addEventListener("unhandledrejection", (event) => {
    logUiError("window.unhandledrejection", event.reason);
  });
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

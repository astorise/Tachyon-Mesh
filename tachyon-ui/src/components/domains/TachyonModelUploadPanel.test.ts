import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { resilientInvoke } from "../../utils/network";
import { listen } from "@tauri-apps/api/event";
import { invoke as coreInvoke } from "@tauri-apps/api/core";
import { setLanguage } from "../../utils/i18n";

vi.mock("../../utils/network", () => ({
  resilientInvoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import "./TachyonModelUploadPanel";
import type { TachyonModelUploadPanel } from "./TachyonModelUploadPanel";

const invokeMock = vi.mocked(resilientInvoke);
const listenMock = vi.mocked(listen);
const previewPayload = {
  alias: "Qwen",
  filesIncluded: 15,
  bytesIncluded: 23 * 1024 * 1024 * 1024,
  files: ["config.json", "model-00001-of-00003.safetensors"],
};

function mountPanel(): TachyonModelUploadPanel {
  const el = document.createElement("tachyon-model-upload-panel") as TachyonModelUploadPanel;
  document.body.appendChild(el);
  return el;
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

beforeEach(() => {
  setLanguage("en");
  invokeMock.mockReset();
  listenMock.mockReset();
  // Default: listen returns an unlisten fn.
  listenMock.mockResolvedValue(vi.fn());
});

afterEach(() => {
  document.body.replaceChildren();
});

describe("TachyonModelUploadPanel", () => {
  it("uploads via the privileged wrapper, never a bare core invoke", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "pick_model_file") return Promise.resolve("/models/llama.gguf");
      if (command === "preview_large_model_upload") return Promise.resolve(previewPayload);
      if (command === "push_large_model") return Promise.resolve("gguf/llama");
      return Promise.reject(new Error(`unexpected ${command}`));
    });

    const panel = mountPanel();
    await panel.selectAndUpload();

    expect(invokeMock).toHaveBeenCalledWith("pick_model_file");
    expect(invokeMock).toHaveBeenCalledWith("preview_large_model_upload", { path: "/models/llama.gguf" });
    expect(invokeMock).toHaveBeenCalledWith("push_large_model", { path: "/models/llama.gguf" });
    expect(coreInvoke).not.toHaveBeenCalled();

    const result = panel.shadowRoot?.querySelector("[data-upload-result]");
    expect(result?.textContent).toContain("gguf/llama");
    expect(result?.textContent).toContain("/v1/models");
  });

  it("renders live progress from upload_progress events", async () => {
    const push = deferred<string>();
    let progressCb: ((event: { payload: number }) => void) | undefined;
    listenMock.mockImplementation((event, cb) => {
      if (event === "upload_progress") {
        progressCb = cb as unknown as (event: { payload: number }) => void;
      }
      return Promise.resolve(vi.fn());
    });
    invokeMock.mockImplementation((command: string) => {
      if (command === "pick_model_file") return Promise.resolve("/models/llama.gguf");
      if (command === "preview_large_model_upload") return Promise.resolve(previewPayload);
      if (command === "push_large_model") return push.promise;
      return Promise.reject(new Error(`unexpected ${command}`));
    });

    const panel = mountPanel();
    const done = panel.selectAndUpload();
    await new Promise((resolve) => setTimeout(resolve, 0)); // drain pick + listener registration

    expect(progressCb).toBeDefined();
    progressCb?.({ payload: 42 });

    const bar = panel.shadowRoot?.querySelector("[data-upload-progress] div div") as HTMLElement | null;
    expect(bar?.style.width).toBe("42%");

    push.resolve("gguf/llama");
    await done;
    expect(panel.shadowRoot?.querySelector("[data-upload-result]")?.textContent).toContain("gguf/llama");
  });

  it("renders upload status details from included files and sent bytes", async () => {
    const push = deferred<string>();
    let statusCb: ((event: { payload: Record<string, unknown> }) => void) | undefined;
    listenMock.mockImplementation((event, cb) => {
      if (event === "upload_status") {
        statusCb = cb as unknown as (event: { payload: Record<string, unknown> }) => void;
      }
      return Promise.resolve(vi.fn());
    });
    invokeMock.mockImplementation((command: string) => {
      if (command === "pick_model_file") return Promise.resolve("/models/Qwen");
      if (command === "preview_large_model_upload") return Promise.resolve(previewPayload);
      if (command === "push_large_model") return push.promise;
      return Promise.reject(new Error(`unexpected ${command}`));
    });

    const panel = mountPanel();
    const done = panel.selectAndUpload();
    await new Promise((resolve) => setTimeout(resolve, 0));

    statusCb?.({
      payload: {
        phase: "included",
        percentage: 0,
        alias: "Qwen",
        file: "model-00001-of-00003.safetensors",
        filesIncluded: 4,
        bytesIncluded: 24 * 1024 * 1024,
      },
    });
    statusCb?.({
      payload: {
        phase: "uploading",
        percentage: 50,
        alias: "Qwen",
        filesIncluded: 4,
        bytesIncluded: 24 * 1024 * 1024,
        archiveBytes: 20 * 1024 * 1024,
        uploadedBytes: 10 * 1024 * 1024,
        totalBytes: 20 * 1024 * 1024,
        part: 2,
      },
    });

    const details = panel.shadowRoot?.querySelector("[data-upload-details]");
    expect(details?.textContent).toContain("model-00001-of-00003.safetensors");
    expect(details?.textContent).toContain("4 file");
    expect(details?.textContent).toContain("Sent 10 MiB / 20 MiB");

    push.resolve("models/Qwen");
    await done;
  });

  it("shows the scanned model files before the privileged upload completes", async () => {
    const push = deferred<string>();
    invokeMock.mockImplementation((command: string) => {
      if (command === "pick_model_file") return Promise.resolve("/models/Qwen");
      if (command === "preview_large_model_upload") return Promise.resolve(previewPayload);
      if (command === "push_large_model") return push.promise;
      return Promise.reject(new Error(`unexpected ${command}`));
    });

    const panel = mountPanel();
    const done = panel.selectAndUpload();
    await new Promise((resolve) => setTimeout(resolve, 0));

    const details = panel.shadowRoot?.querySelector("[data-upload-details]");
    expect(details?.textContent).toContain("model-00001-of-00003.safetensors");
    expect(details?.textContent).toContain("15 file");

    push.resolve("models/Qwen");
    await done;
  });

  it("treats a cancelled file selection as a no-op (no upload)", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "pick_model_file") return Promise.resolve(null);
      return Promise.reject(new Error(`unexpected ${command}`));
    });

    const panel = mountPanel();
    await panel.selectAndUpload();

    expect(invokeMock).toHaveBeenCalledWith("pick_model_file");
    expect(invokeMock).not.toHaveBeenCalledWith("push_large_model", expect.anything());
    expect(panel.shadowRoot?.textContent).toContain("cancelled");
  });

  it("shows the error and re-enables the control on failure", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "pick_model_file") return Promise.resolve("/models/llama.gguf");
      if (command === "preview_large_model_upload") return Promise.resolve(previewPayload);
      if (command === "push_large_model") return Promise.reject(new Error("broker unavailable"));
      return Promise.reject(new Error(`unexpected ${command}`));
    });

    const panel = mountPanel();
    await panel.selectAndUpload();

    const result = panel.shadowRoot?.querySelector("[data-upload-result]");
    expect(result?.textContent).toContain("broker unavailable");
    const button = panel.shadowRoot?.getElementById("btn-select-model") as HTMLButtonElement | null;
    expect(button?.disabled).toBe(false);
  });
});

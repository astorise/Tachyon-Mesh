import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { resilientInvoke as invoke } from "../../utils/network";
import { t } from "../../utils/i18n";

type UploadState = "idle" | "uploading" | "success" | "error";

/**
 * `<tachyon-model-upload-panel>` — lets an operator pick a local model folder
 * (weights plus tokenizer) and upload it to the cluster via the model broker.
 * The client tars and gzips the folder on the fly during the upload.
 *
 * The upload goes through `resilientInvoke("push_large_model", …)`, which is a
 * step-up command, so MFA is enforced by the privileged-command wrapper — this
 * panel never calls the Tauri command directly. Progress is reported through
 * the `upload_progress` event emitted by the command. On success the model is
 * registered automatically (broker → guest-openai) and appears in `/v1/models`.
 */
export class TachyonModelUploadPanel extends TachyonConfigDashboard {
  private state: UploadState = "idle";
  private progress = 0;
  private selectedPath: string | null = null;
  private resultMessage = "";
  private unlisten: UnlistenFn | null = null;
  private readonly onLanguageChanged = () => {
    this.render();
    this.bindEvents();
  };

  connectedCallback(): void {
    window.addEventListener("i18n:language-changed", this.onLanguageChanged);
    this.render();
    this.bindEvents();
  }

  disconnectedCallback(): void {
    window.removeEventListener("i18n:language-changed", this.onLanguageChanged);
    void this.stopProgressListener();
  }

  /** Pick a file, then upload it. A no-op file selection (cancel) is benign. */
  async selectAndUpload(): Promise<void> {
    if (this.state === "uploading") {
      return;
    }
    let path: string | null;
    try {
      path = await invoke<string | null>("pick_model_file");
    } catch (error) {
      this.onError(error);
      return;
    }
    if (!path) {
      this.state = "idle";
      this.resultMessage = t("ai.upload.cancelled");
      this.render();
      this.bindEvents();
      return;
    }

    this.selectedPath = path;
    this.state = "uploading";
    this.progress = 0;
    this.resultMessage = "";
    this.render();
    this.bindEvents();

    await this.startProgressListener();
    try {
      const assetRef = await invoke<string>("push_large_model", { path });
      this.onSuccess(assetRef);
    } catch (error) {
      this.onError(error);
    } finally {
      await this.stopProgressListener();
    }
  }

  /** Update the progress indicator from an `upload_progress` event payload. */
  onProgress(percentage: number): void {
    if (this.state !== "uploading") {
      return;
    }
    this.progress = Math.max(0, Math.min(100, Math.round(percentage)));
    this.render();
    this.bindEvents();
  }

  private onSuccess(assetRef: string): void {
    this.state = "success";
    this.progress = 100;
    this.resultMessage = t("ai.upload.success").replace("{asset}", assetRef);
    this.render();
    this.bindEvents();
  }

  private onError(error: unknown): void {
    const message = error instanceof Error ? error.message : String(error);
    this.state = "error";
    this.resultMessage = t("ai.upload.error").replace("{message}", message);
    this.render();
    this.bindEvents();
  }

  private async startProgressListener(): Promise<void> {
    await this.stopProgressListener();
    this.unlisten = await listen<number>("upload_progress", (event) => {
      this.onProgress(Number(event.payload));
    });
  }

  private async stopProgressListener(): Promise<void> {
    if (this.unlisten) {
      const unlisten = this.unlisten;
      this.unlisten = null;
      await unlisten();
    }
  }

  private render(): void {
    const uploading = this.state === "uploading";
    const selectedLine = this.selectedPath
      ? `<p class="font-mono text-[11px] text-slate-500" data-upload-selected>${t("ai.upload.selected").replace("{path}", this.selectedPath)}</p>`
      : "";

    this.renderTemplate(`
      <section class="space-y-4 rounded border border-slate-700 bg-slate-800/70 p-5 text-slate-300">
        <header>
          <h3 class="text-sm font-bold uppercase tracking-widest text-slate-200">${t("ai.upload.title")}</h3>
          <p class="text-[11px] text-slate-500">${t("ai.upload.subtitle")}</p>
        </header>

        <button id="btn-select-model" type="button" ${uploading ? "disabled" : ""}
          class="w-full rounded border border-cyan-500 bg-transparent py-3 text-sm font-bold text-cyan-400 transition-colors hover:bg-cyan-500 hover:text-slate-950 disabled:cursor-not-allowed disabled:opacity-50">
          ${t("ai.upload.select")}
        </button>

        ${selectedLine}

        ${uploading ? this.renderProgress() : ""}
        ${this.renderResult()}
      </section>
    `);
  }

  private renderProgress(): string {
    return `
      <div data-upload-progress>
        <div class="mb-1 flex justify-between text-[11px]">
          <span class="text-slate-400">${t("ai.upload.uploading").replace("{pct}", String(this.progress))}</span>
        </div>
        <div class="h-2 w-full overflow-hidden rounded-full bg-slate-700">
          <div class="h-full rounded-full bg-cyan-400 transition-all" style="width:${this.progress}%"></div>
        </div>
      </div>
    `;
  }

  private renderResult(): string {
    if (this.state === "success") {
      return `
        <div data-upload-result class="space-y-1 rounded border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-[11px] text-emerald-300">
          <p>${this.resultMessage}</p>
          <p class="text-emerald-400/80">${t("ai.upload.registryHint")}</p>
        </div>
      `;
    }
    if (this.state === "error") {
      return `<div data-upload-result class="rounded border border-red-500/30 bg-red-500/10 px-3 py-2 text-[11px] text-red-300">${this.resultMessage}</div>`;
    }
    if (this.resultMessage) {
      return `<div data-upload-result class="text-[11px] text-slate-500">${this.resultMessage}</div>`;
    }
    return "";
  }

  private bindEvents(): void {
    this.root.getElementById("btn-select-model")?.addEventListener("click", () => {
      void this.selectAndUpload();
    });
  }
}

customElements.define("tachyon-model-upload-panel", TachyonModelUploadPanel);

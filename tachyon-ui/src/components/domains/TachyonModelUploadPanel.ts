import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { resilientInvoke as invoke } from "../../utils/network";
import { t } from "../../utils/i18n";

type UploadState = "idle" | "uploading" | "success" | "error";
type UploadStatus = {
  phase?: string;
  percentage?: number;
  alias?: string | null;
  file?: string | null;
  filesIncluded?: number;
  bytesIncluded?: number;
  archiveBytes?: number | null;
  uploadedBytes?: number | null;
  totalBytes?: number | null;
  part?: number | null;
};

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
  private statusMessage = "";
  private includedFiles: string[] = [];
  private uploadStatus: UploadStatus | null = null;
  private unlisten: UnlistenFn[] = [];
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
    this.statusMessage = t("ai.upload.scanning");
    this.includedFiles = [];
    this.uploadStatus = null;
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

  onStatus(status: UploadStatus): void {
    if (this.state !== "uploading") {
      return;
    }
    this.uploadStatus = status;
    if (typeof status.percentage === "number") {
      this.progress = Math.max(0, Math.min(100, Math.round(status.percentage)));
    }
    if (status.file && !this.includedFiles.includes(status.file)) {
      this.includedFiles = [...this.includedFiles, status.file].slice(-8);
    }
    this.statusMessage = this.describeStatus(status);
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
    const progressUnlisten = await listen<number>("upload_progress", (event) => {
      this.onProgress(Number(event.payload));
    });
    const statusUnlisten = await listen<UploadStatus>("upload_status", (event) => {
      this.onStatus(event.payload);
    });
    this.unlisten = [progressUnlisten, statusUnlisten];
  }

  private async stopProgressListener(): Promise<void> {
    const listeners = this.unlisten;
    this.unlisten = [];
    for (const unlisten of listeners) {
      await unlisten();
    }
  }

  private render(): void {
    const uploading = this.state === "uploading";
    const selectedLine = this.selectedPath
      ? `<p class="font-mono text-[11px] text-slate-500" data-upload-selected>${this.escape(t("ai.upload.selected").replace("{path}", this.selectedPath))}</p>`
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
        ${uploading ? this.renderUploadDetails() : ""}
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

  private renderUploadDetails(): string {
    const status = this.uploadStatus;
    const files = Number(status?.filesIncluded ?? 0);
    const bytes = Number(status?.bytesIncluded ?? 0);
    const archiveBytes = status?.archiveBytes ?? status?.totalBytes ?? null;
    const uploadedBytes = status?.uploadedBytes ?? null;
    const fileRows = this.includedFiles
      .map((file) => `<li class="truncate font-mono text-[10px] text-slate-400" title="${this.escape(file)}">${this.escape(file)}</li>`)
      .join("");
    return `
      <div data-upload-details class="space-y-2 rounded border border-slate-700 bg-slate-900/70 px-3 py-2 text-[11px] text-slate-400">
        <div class="flex flex-wrap items-center justify-between gap-2">
          <span class="text-slate-300">${this.escape(this.statusMessage || t("ai.upload.scanning"))}</span>
          ${status?.part ? `<span class="font-mono text-cyan-300">${t("ai.upload.part").replace("{part}", String(status.part))}</span>` : ""}
        </div>
        <div class="grid gap-1 sm:grid-cols-3">
          <span>${t("ai.upload.files").replace("{count}", String(files))}</span>
          <span>${t("ai.upload.inputSize").replace("{size}", this.formatBytes(bytes))}</span>
          <span>${t("ai.upload.archiveSize").replace("{size}", this.formatBytes(archiveBytes))}</span>
        </div>
        ${uploadedBytes !== null ? `<p>${t("ai.upload.sent").replace("{sent}", this.formatBytes(uploadedBytes)).replace("{total}", this.formatBytes(status?.totalBytes ?? archiveBytes))}</p>` : ""}
        ${fileRows ? `<ul class="max-h-28 space-y-1 overflow-auto border-t border-slate-800 pt-2">${fileRows}</ul>` : ""}
      </div>
    `;
  }

  private describeStatus(status: UploadStatus): string {
    const phase = status.phase ?? "";
    if (phase === "included") {
      return t("ai.upload.including").replace("{file}", status.file ?? "");
    }
    if (phase === "prepared") {
      return t("ai.upload.prepared")
        .replace("{count}", String(status.filesIncluded ?? 0))
        .replace("{size}", this.formatBytes(status.archiveBytes ?? status.totalBytes));
    }
    if (phase === "uploading") {
      return t("ai.upload.sending");
    }
    if (phase === "committing") {
      return t("ai.upload.committing");
    }
    return t("ai.upload.scanning");
  }

  private formatBytes(value: number | null | undefined): string {
    if (!Number.isFinite(value ?? NaN) || (value ?? 0) <= 0) {
      return "0 B";
    }
    const units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let size = Number(value);
    let unit = 0;
    while (size >= 1024 && unit < units.length - 1) {
      size /= 1024;
      unit += 1;
    }
    return `${size >= 10 || unit === 0 ? size.toFixed(0) : size.toFixed(1)} ${units[unit]}`;
  }

  private escape(value: string): string {
    return value
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
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

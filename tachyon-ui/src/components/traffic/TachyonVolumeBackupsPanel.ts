import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { resilientInvoke as invoke } from "../../utils/network";

type BackupSnapshot = {
  snapshotId: string;
  routePath: string;
  guestPath: string;
  timestampMs: number;
  objectCount: number;
};

export class TachyonVolumeBackupsPanel extends TachyonConfigDashboard {
  private routePath = "";
  private guestPath = "";
  private snapshots: BackupSnapshot[] = [];
  private feedback = "";
  private feedbackOk = true;

  static get observedAttributes() {
    return ["route-path", "guest-path"];
  }

  attributeChangedCallback(name: string, _old: string, value: string): void {
    if (name === "route-path") this.routePath = value;
    if (name === "guest-path") this.guestPath = value;
  }

  connectedCallback(): void {
    this.routePath = this.getAttribute("route-path") ?? "";
    this.guestPath = this.getAttribute("guest-path") ?? "";
    this.render();
    this.bindEvents();
    void this.load();
  }

  private async load(): Promise<void> {
    await this.withLoadingState(async () => {
      this.snapshots = await invoke<BackupSnapshot[]>("list_volume_backups", {
        routePath: this.routePath,
        guestPath: this.guestPath,
      });
      this.render();
      this.bindEvents();
    });
  }

  private render(): void {
    const guestLabel = this.escHtml(this.guestPath);
    this.renderTemplate(`
      <section class="mt-2 rounded-lg border border-slate-700/60 bg-slate-900/60 p-3 space-y-2">
        <header class="flex items-center justify-between">
          <h5 class="text-xs font-bold uppercase tracking-widest text-slate-500">
            Backups — <span class="font-mono text-slate-400">${guestLabel}</span>
          </h5>
          <button id="btn-backup-now"
            class="rounded border border-emerald-700/50 bg-emerald-800/20 px-2 py-1 text-xs text-emerald-300 hover:bg-emerald-800/30">
            Backup now
          </button>
        </header>

        ${this.feedback ? `
          <p class="rounded px-3 py-1.5 text-xs font-mono ${this.feedbackOk ? "bg-emerald-900/40 text-emerald-300" : "bg-red-900/40 text-red-300"}">
            ${this.escHtml(this.feedback)}
          </p>` : ""}

        ${this.snapshots.length === 0
          ? `<p class="text-xs text-slate-500 italic">No snapshots yet. Click "Backup now" to create the first one.</p>`
          : `<ul class="space-y-1" id="backup-list">
              ${this.snapshots.map((s) => this.renderSnapshotRow(s)).join("")}
            </ul>`}
      </section>
    `);
  }

  private renderSnapshotRow(s: BackupSnapshot): string {
    const date = new Date(s.timestampMs).toLocaleString();
    return `
      <li class="flex items-center justify-between gap-2 rounded border border-slate-700/40 bg-slate-800/40 px-3 py-1.5"
          data-snapshot-id="${this.escHtml(s.snapshotId)}">
        <div class="min-w-0">
          <p class="font-mono text-xs text-slate-300 truncate">${this.escHtml(s.snapshotId)}</p>
          <p class="text-xs text-slate-500">${date} · ${s.objectCount} objects</p>
        </div>
        <button data-restore-id="${this.escHtml(s.snapshotId)}"
          class="shrink-0 rounded border border-amber-700/50 bg-amber-800/20 px-2 py-1 text-xs text-amber-300 hover:bg-amber-800/30">
          Restore
        </button>
      </li>`;
  }

  private bindEvents(): void {
    this.root.getElementById("btn-backup-now")?.addEventListener("click", () => {
      void this.triggerBackup();
    });

    this.root.querySelectorAll<HTMLButtonElement>("[data-restore-id]").forEach((btn) => {
      btn.addEventListener("click", () => {
        const snapshotId = btn.dataset.restoreId ?? "";
        if (!snapshotId) return;
        if (!window.confirm(`Restore volume "${this.guestPath}" on route "${this.routePath}" from snapshot "${snapshotId}"?\n\nThis will overwrite the current volume contents.`)) return;
        void this.triggerRestore(snapshotId);
      });
    });
  }

  private async triggerBackup(): Promise<void> {
    try {
      const snapshot = await invoke<BackupSnapshot>("backup_volume", {
        routePath: this.routePath,
        guestPath: this.guestPath,
      });
      this.feedback = `Backup created: ${snapshot.snapshotId} (${snapshot.objectCount} objects).`;
      this.feedbackOk = true;
      await this.load();
    } catch (error) {
      this.feedback = error instanceof Error ? error.message : String(error);
      this.feedbackOk = false;
      this.render();
      this.bindEvents();
    }
  }

  private async triggerRestore(snapshotId: string): Promise<void> {
    try {
      await invoke<void>("restore_volume", {
        routePath: this.routePath,
        guestPath: this.guestPath,
        snapshotId,
      });
      this.feedback = `Volume restored from snapshot ${snapshotId}.`;
      this.feedbackOk = true;
      this.render();
      this.bindEvents();
    } catch (error) {
      this.feedback = error instanceof Error ? error.message : String(error);
      this.feedbackOk = false;
      this.render();
      this.bindEvents();
    }
  }

  private escHtml(str: string): string {
    return str.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
  }
}

customElements.define("tachyon-volume-backups-panel", TachyonVolumeBackupsPanel);

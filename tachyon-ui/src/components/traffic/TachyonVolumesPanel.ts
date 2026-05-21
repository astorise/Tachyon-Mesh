import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { resilientInvoke as invoke } from "../../utils/network";

type S3VolumeEntry = {
  s3Url: string;
  guestPath: string;
  readonly: boolean;
};

const S3_URL_RE = /^s3:\/\/[^/]+/;

export class TachyonVolumesPanel extends TachyonConfigDashboard {
  private routePath = "";
  private volumes: S3VolumeEntry[] = [];
  private addingVolume = false;
  private validationError = "";
  private feedback = "";
  private feedbackOk = true;

  static get observedAttributes() {
    return ["route-path"];
  }

  attributeChangedCallback(name: string, _old: string, value: string): void {
    if (name === "route-path") {
      this.routePath = value;
    }
  }

  connectedCallback(): void {
    if (this.hasAttribute("route-path")) {
      this.routePath = this.getAttribute("route-path") ?? "";
    }
    this.render();
    this.bindEvents();
    void this.load();
  }

  private async load(): Promise<void> {
    await this.withLoadingState(async () => {
      const raw = await invoke<S3VolumeEntry[]>("list_s3_volumes", { routePath: this.routePath });
      this.volumes = raw;
      this.render();
      this.bindEvents();
    });
  }

  private render(): void {
    this.renderTemplate(`
      <section class="mt-3 rounded-lg border border-slate-700 bg-slate-900/80 p-4 space-y-3">
        <header class="flex items-center justify-between">
          <h4 class="text-xs font-bold uppercase tracking-widest text-slate-400">Volumes</h4>
          ${this.addingVolume ? "" : `
            <button id="btn-add-s3"
              class="rounded border border-cyan-600/50 bg-cyan-700/20 px-2 py-1 text-xs text-cyan-300 hover:bg-cyan-700/30">
              + Add S3 Volume
            </button>`}
        </header>

        ${this.feedback ? `
          <p class="rounded px-3 py-2 text-xs font-mono ${this.feedbackOk ? "bg-emerald-900/40 text-emerald-300" : "bg-red-900/40 text-red-300"}">
            ${this.escHtml(this.feedback)}
          </p>` : ""}

        ${this.volumes.length === 0 && !this.addingVolume
          ? `<p class="text-xs text-slate-500 italic">No volumes configured. Add an S3 volume to mount object-store data into the guest.</p>`
          : ""}

        <div id="volume-cards" class="space-y-2">
          ${this.volumes.map((v) => this.renderVolumeCard(v)).join("")}
        </div>

        ${this.addingVolume ? this.renderAddForm() : ""}
      </section>
    `);
  }

  private renderVolumeCard(v: S3VolumeEntry): string {
    const mode = v.readonly
      ? `<span class="rounded bg-amber-900/40 px-1.5 py-0.5 text-xs text-amber-300">read-only</span>`
      : `<span class="rounded bg-cyan-900/40 px-1.5 py-0.5 text-xs text-cyan-300">read-write</span>`;
    return `
      <article class="flex items-start justify-between gap-3 rounded border border-slate-700/80 bg-slate-800/60 px-3 py-2" data-guest-path="${this.escHtml(v.guestPath)}">
        <div class="min-w-0 space-y-0.5">
          <div class="flex items-center gap-2">
            <span class="text-xs text-slate-500 font-mono">S3</span>
            ${mode}
          </div>
          <p class="font-mono text-xs text-cyan-200 truncate">${this.escHtml(v.s3Url)}</p>
          <p class="font-mono text-xs text-slate-400">${this.escHtml(v.guestPath)}</p>
        </div>
        <button data-remove-path="${this.escHtml(v.guestPath)}"
          class="shrink-0 rounded border border-red-800/60 bg-red-900/20 px-2 py-1 text-xs text-red-400 hover:bg-red-900/40">
          Remove
        </button>
      </article>`;
  }

  private renderAddForm(): string {
    return `
      <form id="add-s3-form" class="space-y-3 rounded border border-slate-700 bg-slate-800/60 p-3">
        <p class="text-xs font-bold uppercase tracking-widest text-slate-400">New S3 Volume</p>

        <div class="space-y-1">
          <label class="block text-xs text-slate-400" for="s3-url">S3 URL</label>
          <input id="s3-url" type="text" placeholder="s3://bucket/prefix"
            class="w-full rounded border border-slate-600 bg-slate-900 px-3 py-1.5 font-mono text-xs text-slate-200 focus:border-cyan-500 focus:outline-none" />
          ${this.validationError ? `<p class="text-xs text-red-400">${this.escHtml(this.validationError)}</p>` : ""}
        </div>

        <div class="space-y-1">
          <label class="block text-xs text-slate-400" for="s3-guest-path">Guest mount path</label>
          <input id="s3-guest-path" type="text" placeholder="/app/data"
            class="w-full rounded border border-slate-600 bg-slate-900 px-3 py-1.5 font-mono text-xs text-slate-200 focus:border-cyan-500 focus:outline-none" />
        </div>

        <label class="flex items-center gap-2 text-xs text-slate-400 cursor-pointer">
          <input id="s3-readonly" type="checkbox" class="rounded" />
          Read-only (no upload after execution)
        </label>

        <div class="flex gap-2">
          <button id="btn-submit-s3" type="submit"
            class="rounded border border-cyan-600/60 bg-cyan-700/30 px-3 py-1.5 text-xs text-cyan-200 hover:bg-cyan-700/40 disabled:opacity-40 disabled:cursor-not-allowed">
            Attach
          </button>
          <button id="btn-cancel-s3" type="button"
            class="rounded border border-slate-600 bg-slate-800 px-3 py-1.5 text-xs text-slate-400 hover:bg-slate-700">
            Cancel
          </button>
        </div>
      </form>`;
  }

  private bindEvents(): void {
    this.root.getElementById("btn-add-s3")?.addEventListener("click", () => {
      this.addingVolume = true;
      this.validationError = "";
      this.render();
      this.bindEvents();
      this.root.getElementById("s3-url")?.focus();
    });

    this.root.getElementById("btn-cancel-s3")?.addEventListener("click", () => {
      this.addingVolume = false;
      this.validationError = "";
      this.render();
      this.bindEvents();
    });

    const s3UrlInput = this.root.getElementById("s3-url") as HTMLInputElement | null;
    const submitBtn = this.root.getElementById("btn-submit-s3") as HTMLButtonElement | null;
    s3UrlInput?.addEventListener("input", () => {
      const val = s3UrlInput.value.trim();
      const valid = S3_URL_RE.test(val);
      this.validationError = val.length > 0 && !valid
        ? "S3 URL must match s3://bucket/prefix"
        : "";
      if (submitBtn) submitBtn.disabled = val.length === 0 || !valid;
      const errEl = this.root.querySelector<HTMLElement>("#add-s3-form p.text-red-400");
      if (errEl) errEl.textContent = this.validationError;
    });

    this.root.getElementById("add-s3-form")?.addEventListener("submit", (e) => {
      e.preventDefault();
      void this.submitAdd();
    });

    this.root.querySelectorAll<HTMLButtonElement>("[data-remove-path]").forEach((btn) => {
      btn.addEventListener("click", () => {
        const guestPath = btn.dataset.removePath ?? "";
        if (!guestPath) return;
        if (!window.confirm(`Remove S3 volume at "${guestPath}" from route "${this.routePath}"?`)) return;
        void this.removeVolume(guestPath);
      });
    });
  }

  private async submitAdd(): Promise<void> {
    const s3Url = (this.root.getElementById("s3-url") as HTMLInputElement | null)?.value.trim() ?? "";
    const guestPath = (this.root.getElementById("s3-guest-path") as HTMLInputElement | null)?.value.trim() ?? "";
    const readonly = (this.root.getElementById("s3-readonly") as HTMLInputElement | null)?.checked ?? false;

    if (!S3_URL_RE.test(s3Url)) {
      this.validationError = "S3 URL must match s3://bucket/prefix";
      this.render();
      this.bindEvents();
      return;
    }

    if (!guestPath.startsWith("/")) {
      this.validationError = "Guest path must be absolute (start with /)";
      this.render();
      this.bindEvents();
      return;
    }

    try {
      await invoke<S3VolumeEntry>("attach_s3_volume", { routePath: this.routePath, s3Url, guestPath, readonly });
      this.feedback = `Volume "${s3Url}" attached at "${guestPath}".`;
      this.feedbackOk = true;
      this.addingVolume = false;
      this.validationError = "";
      await this.load();
    } catch (error) {
      this.feedback = error instanceof Error ? error.message : String(error);
      this.feedbackOk = false;
      this.render();
      this.bindEvents();
    }
  }

  private async removeVolume(guestPath: string): Promise<void> {
    try {
      await invoke<void>("detach_s3_volume", { routePath: this.routePath, guestPath });
      this.feedback = `Volume at "${guestPath}" detached.`;
      this.feedbackOk = true;
      await this.load();
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

customElements.define("tachyon-volumes-panel", TachyonVolumesPanel);

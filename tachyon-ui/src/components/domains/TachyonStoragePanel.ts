import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { applyAndSeal, resilientInvoke as invoke } from "../../utils/network";
import { t } from "../../utils/i18n";

type MeshResource = {
  name: string;
  type: string;
  target: string;
  pending?: boolean;
};

export class TachyonStoragePanel extends TachyonConfigDashboard {
  private resources: MeshResource[] = [];

  async connectedCallback(): Promise<void> {
    this.render();
    this.bindForm();
    this.animateEntrance();
    try {
      this.resources = await invoke<MeshResource[]>("get_resources");
    } catch {
      this.resources = [];
    }
    this.render();
    this.bindForm();
  }

  private bindForm(): void {
    this.root.querySelector("form")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.applyStorageConfig();
    });
  }

  private render(): void {
    this.renderTemplate(`
      <section class="p-6 space-y-6 text-slate-300">
        <header data-stagger-panel class="border-l-4 border-cyan-500 pl-4">
          <h2 class="text-2xl font-bold text-slate-100">Storage Volumes</h2>
          <p class="text-sm font-mono text-slate-400">Domain: config-storage / WASI state</p>
        </header>

        <article data-stagger-panel class="rounded-lg border border-slate-800 bg-slate-900/70 p-5">
          <h3 class="mb-3 text-sm font-semibold uppercase tracking-widest text-cyan-300">${t("storage.preview.title")}</h3>
          ${this.renderResources()}
        </article>

        <form class="space-y-6 rounded-lg border border-slate-700 bg-slate-800/40 p-6">
          <label data-stagger-panel class="block text-xs uppercase tracking-widest text-cyan-500">WASI Volume Mount Path
            <input id="mount-path" type="text" value="/mnt/data" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm text-slate-200 outline-none transition-colors focus:border-cyan-400">
          </label>

          <label data-stagger-panel class="block text-xs uppercase tracking-widest text-cyan-500">S3 Proxy Endpoint
            <input id="s3-endpoint" type="url" placeholder="https://s3-proxy.tachyon.local" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm text-slate-200 outline-none transition-colors focus:border-cyan-400">
          </label>

          <button data-stagger-panel class="border border-cyan-500 px-6 py-3 font-bold text-cyan-500 transition-colors hover:bg-cyan-500 hover:text-slate-950">
            Apply Storage Config
          </button>
        </form>

        <div id="feedback-zone" data-stagger-panel class="rounded-xl border border-slate-800 bg-slate-900 px-4 py-3 font-mono text-xs text-slate-400">Awaiting storage configuration.</div>
      </section>
    `);
  }

  private renderResources(): string {
    if (this.resources.length === 0) {
      return `<p class="text-xs text-slate-500">${t("storage.preview.empty")}</p>`;
    }
    const rows = this.resources
      .map(
        (resource) =>
          `<tr><td class="py-1 pr-4 text-cyan-300">${this.escape(resource.name)}</td><td class="py-1 pr-4 text-slate-300">${this.escape(resource.type)}</td><td class="py-1 pr-4 font-mono text-slate-300">${this.escape(resource.target)}</td><td class="py-1 text-slate-300">${resource.pending ? "yes" : "no"}</td></tr>`,
      )
      .join("");
    return `
      <table class="w-full text-xs">
        <thead class="text-slate-500 uppercase tracking-widest">
          <tr><th class="text-left pb-2 pr-4">${t("storage.preview.column.name")}</th><th class="text-left pb-2 pr-4">${t("storage.preview.column.kind")}</th><th class="text-left pb-2 pr-4">${t("storage.preview.column.target")}</th><th class="text-left pb-2">${t("storage.preview.column.pending")}</th></tr>
        </thead>
        <tbody>${rows}</tbody>
      </table>
    `;
  }

  private async applyStorageConfig(): Promise<void> {
    try {
      const response = await applyAndSeal("storage", {
          mount_path: this.value("mount-path", "/mnt/data"),
          s3_endpoint: this.value("s3-endpoint", ""),
      });
      this.showFeedback(response.success ? "success" : "error", response.message);
    } catch (error) {
      this.showFeedback("error", error instanceof Error ? error.message : String(error));
    }
  }

  private value(id: string, fallback: string): string {
    const value = (this.root.getElementById(id) as HTMLInputElement | null)?.value.trim();
    return value ? value : fallback;
  }

  private escape(value: string): string {
    return value
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#039;");
  }
}

customElements.define("tachyon-storage-panel", TachyonStoragePanel);

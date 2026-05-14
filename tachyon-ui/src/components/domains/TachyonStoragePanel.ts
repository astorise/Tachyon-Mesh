import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { applyAndSeal, resilientInvoke as invoke } from "../../utils/network";
import { t } from "../../utils/i18n";

type MeshResource = {
  name: string;
  type: string;
  target: string;
  pending?: boolean;
};

type KvResult = {
  namespace: string;
  key: string;
  value: string | null;
  error: string | null;
};

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#039;");
}

export class TachyonStoragePanel extends TachyonConfigDashboard {
  private resources: MeshResource[] = [];
  private kvResult: KvResult | null = null;
  private kvBusy = false;
  private readonly onLanguageChanged = () => { this.render(); this.bindForm(); this.bindKvForm(); };

  async connectedCallback(): Promise<void> {
    window.addEventListener("i18n:language-changed", this.onLanguageChanged);
    this.render();
    this.bindForm();
    this.bindKvForm();
    this.animateEntrance();
    try {
      this.resources = await invoke<MeshResource[]>("get_resources");
    } catch {
      this.resources = [];
    }
    this.render();
    this.bindForm();
    this.bindKvForm();
  }

  disconnectedCallback(): void {
    window.removeEventListener("i18n:language-changed", this.onLanguageChanged);
  }

  private bindForm(): void {
    this.root.querySelector("#storage-config-form")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.applyStorageConfig();
    });
  }

  private bindKvForm(): void {
    this.root.getElementById("btn-kv-get")?.addEventListener("click", () => void this.kvGet());
    this.root.getElementById("btn-kv-delete")?.addEventListener("click", () => void this.kvDelete());
    this.root.getElementById("btn-kv-inspect-close")?.addEventListener("click", () => {
      this.kvResult = null;
      this.renderKvResult();
    });
  }

  private render(): void {
    this.renderTemplate(`
      <section class="p-6 space-y-6 text-slate-300">
        <header data-stagger-panel class="border-l-4 border-cyan-500 pl-4">
          <h2 class="text-2xl font-bold text-slate-100">Storage &amp; KV Explorer</h2>
          <p class="text-sm font-mono text-slate-400">Domain: config-storage / WASI state / KV-Partition V2</p>
        </header>

        <article data-stagger-panel class="rounded-lg border border-slate-800 bg-slate-900/70 p-5">
          <h3 class="mb-3 text-sm font-semibold uppercase tracking-widest text-cyan-300">${t("storage.preview.title")}</h3>
          ${this.renderResources()}
        </article>

        <!-- KV-Partition V2 Explorer -->
        <article data-stagger-panel class="rounded-lg border border-slate-700 bg-slate-800/40 p-5 space-y-4">
          <h3 class="text-sm font-semibold uppercase tracking-widest text-purple-300">KV-Partition V2 Explorer</h3>
          <div class="grid grid-cols-1 gap-3 md:grid-cols-2">
            <label class="block text-xs uppercase tracking-widest text-cyan-500">Namespace
              <input id="kv-namespace" type="text" placeholder="e.g. sessions" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm font-mono text-slate-200 outline-none transition-colors focus:border-cyan-400">
            </label>
            <label class="block text-xs uppercase tracking-widest text-cyan-500">Key
              <input id="kv-key" type="text" placeholder="e.g. user:alice:prefs" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm font-mono text-slate-200 outline-none transition-colors focus:border-cyan-400">
            </label>
          </div>
          <div class="flex gap-3">
            <button id="btn-kv-get" type="button" class="rounded border border-cyan-500/50 bg-cyan-500/10 px-4 py-2 text-xs font-semibold uppercase tracking-wider text-cyan-300 transition-colors hover:bg-cyan-500/20 disabled:cursor-wait disabled:opacity-50">
              Get
            </button>
            <button id="btn-kv-delete" type="button" class="rounded border border-red-500/40 bg-red-500/10 px-4 py-2 text-xs font-semibold uppercase tracking-wider text-red-300 transition-colors hover:bg-red-500/20 disabled:cursor-wait disabled:opacity-50">
              Delete
            </button>
          </div>
          <div id="kv-result-zone" class="min-h-[3rem]"></div>
        </article>

        <form id="storage-config-form" class="space-y-6 rounded-lg border border-slate-700 bg-slate-800/40 p-6">
          <label data-stagger-panel class="block text-xs uppercase tracking-widest text-cyan-500">WASI Volume Mount Path
            <input id="mount-path" type="text" value="/mnt/data" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm text-slate-200 outline-none transition-colors focus:border-cyan-400">
          </label>

          <label data-stagger-panel class="block text-xs uppercase tracking-widest text-cyan-500">S3 Proxy Endpoint
            <input id="s3-endpoint" type="url" placeholder="https://s3-proxy.tachyon.local" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm text-slate-200 outline-none transition-colors focus:border-cyan-400">
          </label>

          <button data-stagger-panel type="submit" class="border border-cyan-500 px-6 py-3 font-bold text-cyan-500 transition-colors hover:bg-cyan-500 hover:text-slate-950">
            Apply Storage Config
          </button>
        </form>

        <div id="feedback-zone" data-stagger-panel class="rounded-xl border border-slate-800 bg-slate-900 px-4 py-3 font-mono text-xs text-slate-400">Awaiting storage configuration.</div>
      </section>
    `);
    this.renderKvResult();
  }

  private renderKvResult(): void {
    const zone = this.root.getElementById("kv-result-zone");
    if (!zone) return;
    if (!this.kvResult) {
      zone.replaceChildren();
      return;
    }
    const r = this.kvResult;
    const onClose = () => {
      this.kvResult = null;
      this.renderKvResult();
    };

    // Build the container using DOM APIs so user-controlled values go through
    // textContent and never reach innerHTML — no escaping needed.
    const container = document.createElement("div");
    const closeBtn = document.createElement("button");
    closeBtn.id = "btn-kv-inspect-close";
    closeBtn.type = "button";
    closeBtn.textContent = "✕";
    closeBtn.addEventListener("click", onClose);

    if (r.error) {
      container.className =
        "rounded border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300 font-mono flex items-start justify-between gap-2";
      const msg = document.createElement("span");
      msg.textContent = r.error;
      closeBtn.className = "text-slate-500 hover:text-slate-300 shrink-0";
      container.append(msg, closeBtn);
    } else if (r.value === null) {
      container.className =
        "rounded border border-slate-600 bg-slate-800/50 px-3 py-2 text-xs text-slate-400 font-mono flex items-start justify-between gap-2";
      const msg = document.createElement("span");
      msg.textContent = "(key not found)";
      closeBtn.className = "text-slate-500 hover:text-slate-300 shrink-0";
      container.append(msg, closeBtn);
    } else {
      let display = r.value;
      try { display = JSON.stringify(JSON.parse(r.value), null, 2); } catch { /* raw */ }
      container.className = "rounded border border-emerald-500/30 bg-emerald-500/5 p-3 space-y-1";
      const header = document.createElement("div");
      header.className = "flex items-center justify-between";
      const meta = document.createElement("span");
      meta.className = "text-[10px] font-mono text-emerald-400/70";
      meta.textContent = `${r.namespace} / ${r.key} · ${r.value.length} bytes`;
      closeBtn.className = "text-slate-500 hover:text-slate-300 text-xs";
      header.append(meta, closeBtn);
      const pre = document.createElement("pre");
      pre.className = "text-xs text-slate-300 font-mono whitespace-pre-wrap break-all max-h-48 overflow-y-auto";
      pre.textContent = display;
      container.append(header, pre);
    }

    zone.replaceChildren(container);
  }

  private renderResources(): string {
    if (this.resources.length === 0) {
      return `<p class="text-xs text-slate-500">${t("storage.preview.empty")}</p>`;
    }
    const rows = this.resources
      .map(
        (resource) =>
          `<tr><td class="py-1 pr-4 text-cyan-300">${escapeHtml(resource.name)}</td><td class="py-1 pr-4 text-slate-300">${escapeHtml(resource.type)}</td><td class="py-1 pr-4 font-mono text-slate-300">${escapeHtml(resource.target)}</td><td class="py-1 text-slate-300">${resource.pending ? "yes" : "no"}</td></tr>`,
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

  private async kvGet(): Promise<void> {
    const ns = this.kvInputValue("kv-namespace");
    const key = this.kvInputValue("kv-key");
    if (!ns || !key) {
      this.kvResult = { namespace: ns, key, value: null, error: "Namespace and key are required." };
      this.renderKvResult();
      return;
    }
    this.setKvBusy(true);
    try {
      const raw = await invoke<string | null>("kv_get", { namespace: ns, key });
      this.kvResult = { namespace: ns, key, value: raw, error: null };
    } catch (error) {
      this.kvResult = { namespace: ns, key, value: null, error: error instanceof Error ? error.message : String(error) };
    } finally {
      this.setKvBusy(false);
    }
    this.renderKvResult();
  }

  private async kvDelete(): Promise<void> {
    const ns = this.kvInputValue("kv-namespace");
    const key = this.kvInputValue("kv-key");
    if (!ns || !key) {
      this.kvResult = { namespace: ns, key, value: null, error: "Namespace and key are required." };
      this.renderKvResult();
      return;
    }
    this.setKvBusy(true);
    try {
      await invoke("kv_delete", { namespace: ns, key });
      this.kvResult = { namespace: ns, key, value: null, error: null };
      window.dispatchEvent(new CustomEvent("toast", {
        detail: { type: "success", message: `Deleted ${ns}/${key}` },
      }));
    } catch (error) {
      this.kvResult = { namespace: ns, key, value: null, error: error instanceof Error ? error.message : String(error) };
    } finally {
      this.setKvBusy(false);
    }
    this.renderKvResult();
  }

  private setKvBusy(busy: boolean): void {
    this.kvBusy = busy;
    const getBtn = this.root.getElementById("btn-kv-get") as HTMLButtonElement | null;
    const delBtn = this.root.getElementById("btn-kv-delete") as HTMLButtonElement | null;
    if (getBtn) getBtn.disabled = busy;
    if (delBtn) delBtn.disabled = busy;
  }

  private async applyStorageConfig(): Promise<void> {
    try {
      const response = await applyAndSeal("storage", {
          mount_path: this.inputValue("mount-path", "/mnt/data"),
          s3_endpoint: this.inputValue("s3-endpoint", ""),
      });
      this.showFeedback(response.success ? "success" : "error", response.message);
    } catch (error) {
      this.showFeedback("error", error instanceof Error ? error.message : String(error));
    }
  }

  private kvInputValue(id: string): string {
    return (this.root.getElementById(id) as HTMLInputElement | null)?.value.trim() ?? "";
  }

  private inputValue(id: string, fallback: string): string {
    const value = (this.root.getElementById(id) as HTMLInputElement | null)?.value.trim();
    return value ? value : fallback;
  }
}

customElements.define("tachyon-storage-panel", TachyonStoragePanel);

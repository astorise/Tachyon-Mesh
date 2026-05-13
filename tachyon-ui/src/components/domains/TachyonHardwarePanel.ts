import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { applyAndSeal, resilientInvoke as invoke } from "../../utils/network";
import { t } from "../../utils/i18n";

type GpuStats = {
  id: string;
  model: string;
  vramTotalMb: number;
  vramUsedMb: number;
  computeUtilization: number;
};

type HardwareStatus = {
  totalRamMb: number;
  availableRamMb: number;
  accelerators: string[];
  gpus: GpuStats[];
};

type RuntimeMetrics = {
  vramUtilizationPct: number;
  ramOffloadActive: boolean;
};

export class TachyonHardwarePanel extends TachyonConfigDashboard {
  private liveStatus: HardwareStatus | null = null;
  private metrics: RuntimeMetrics | null = null;
  private readonly onLanguageChanged = () => { this.render(); this.bindForm(); };

  async connectedCallback(): Promise<void> {
    window.addEventListener("i18n:language-changed", this.onLanguageChanged);
    this.render();
    this.bindForm();
    this.animateEntrance();
    await this.withLoadingState(async () => {
      const [status, metrics] = await Promise.all([
        invoke<HardwareStatus>("get_hardware_status"),
        invoke<RuntimeMetrics>("get_metrics").catch(() => null),
      ]);
      this.liveStatus = status;
      this.metrics = metrics;
      this.render();
      this.bindForm();
    });
  }

  disconnectedCallback(): void {
    window.removeEventListener("i18n:language-changed", this.onLanguageChanged);
  }

  private render(): void {
    this.renderTemplate(`
      <section class="p-6 space-y-6 text-slate-300">
        <div data-stagger-panel class="border-l-4 border-cyan-500 pl-4 flex flex-col gap-1">
          ${this.liveStatus ? `<span class="text-[10px] font-mono text-emerald-400/80">RAM: ${this.liveStatus.availableRamMb} / ${this.liveStatus.totalRamMb} MiB free · Accelerators: ${this.liveStatus.accelerators.join(", ") || "—"}</span>` : ""}
          <h2 class="text-2xl font-bold text-slate-100">${t("hardware.title")}</h2>
          <p class="text-sm font-mono text-slate-400">${t("hardware.subtitle")}</p>
        </div>

        ${this.renderVramSection()}

        <form class="space-y-6">
          <div data-stagger-panel class="grid grid-cols-1 gap-6 rounded-lg border border-slate-700 bg-slate-800/40 p-6 md:grid-cols-2">
            <label class="block text-xs uppercase tracking-widest text-cyan-500">${t("hardware.field.accelerator")}
              <select id="accelerator" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm text-slate-200 outline-none transition-colors focus:border-cyan-400">
                <option value="npu">${t("hardware.option.npu")}</option>
                <option value="tpu">${t("hardware.option.tpu")}</option>
                <option value="gpu">${t("hardware.option.gpu")}</option>
              </select>
            </label>
            <label class="flex min-h-20 items-center justify-between rounded border border-slate-800 bg-slate-900/50 px-4 text-xs uppercase tracking-widest text-cyan-500">
              <span>${t("hardware.field.ebpf")}</span>
              <input id="xdp-offload" type="checkbox" checked class="h-5 w-5 accent-cyan-500">
            </label>
          </div>

          <button id="apply-hardware" class="bg-cyan-600 px-8 py-3 font-black uppercase tracking-tight text-slate-950 transition-colors hover:bg-cyan-500">
            ${t("hardware.button")}
          </button>
        </form>

        <div id="feedback-zone" data-stagger-panel class="mt-4 rounded-xl border border-slate-800 bg-slate-900 px-4 py-3 font-mono text-xs text-slate-400">${t("hardware.feedback.empty")}</div>
      </section>
    `);
  }

  private renderVramSection(): string {
    const vramPct = this.metrics?.vramUtilizationPct ?? null;
    const ramOffload = this.metrics?.ramOffloadActive ?? false;
    const gpus = this.liveStatus?.gpus ?? [];

    if (vramPct === null && gpus.length === 0) {
      return "";
    }

    const barColor = vramPct !== null
      ? vramPct >= 90 ? "bg-red-500" : vramPct >= 80 ? "bg-amber-400" : "bg-purple-500"
      : "bg-purple-500";

    // Per-GPU bars (when per-GPU data is available)
    const gpuBars = gpus.map((gpu) => {
      const used = gpu.vramUsedMb;
      const total = gpu.vramTotalMb;
      const pct = total > 0 ? Math.round((used / total) * 100) : (vramPct ?? 0);
      const color = pct >= 90 ? "bg-red-500" : pct >= 80 ? "bg-amber-400" : "bg-purple-500";
      const label = total > 0 ? `${used} / ${total} MB` : `${pct}%`;
      return `
        <div class="mb-3">
          <div class="flex justify-between text-xs text-slate-400 mb-1">
            <span class="font-mono">${this.escape(gpu.model)} <span class="text-slate-600">(${this.escape(gpu.id)})</span></span>
            <span class="font-mono">${label}</span>
          </div>
          <div class="w-full bg-slate-700 rounded-full h-2">
            <div class="${color} h-2 rounded-full transition-all" style="width: ${pct}%"></div>
          </div>
        </div>
      `;
    }).join("");

    // Cluster-wide VRAM bar (always shown when we have metrics)
    const clusterBar = vramPct !== null ? `
      <div class="mb-3">
        <div class="flex justify-between text-xs text-slate-400 mb-1">
          <span class="uppercase tracking-widest">${t("ai.vram.utilization")}</span>
          <span class="font-mono">${vramPct}%</span>
        </div>
        <div class="w-full bg-slate-700 rounded-full h-2">
          <div class="${barColor} h-2 rounded-full transition-all" style="width: ${vramPct}%"></div>
        </div>
      </div>
    ` : "";

    const offloadBadge = ramOffload ? `
      <div class="mt-2 rounded border border-amber-500/40 bg-amber-500/10 px-3 py-1.5 text-xs text-amber-300">
        ⚠ ${t("ai.vram.offload.active")} — ${t("ai.vram.offload.desc")}
      </div>
    ` : "";

    return `
      <div data-stagger-panel class="rounded-lg border border-purple-500/30 bg-purple-500/5 p-5 space-y-2">
        <div class="flex items-center justify-between mb-3">
          <h3 class="text-sm font-semibold uppercase tracking-widest text-purple-300">${t("ai.vram.title")}</h3>
          <button id="btn-vram-refresh" type="button" class="text-xs text-slate-500 hover:text-cyan-300 transition-colors">${t("ai.vram.refresh")}</button>
        </div>
        ${gpuBars || clusterBar}
        ${offloadBadge}
      </div>
    `;
  }

  private bindForm(): void {
    this.root.querySelector("form")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.applyHardwarePolicy();
    });
    this.root.getElementById("btn-vram-refresh")?.addEventListener("click", () => {
      void this.refreshVram();
    });
  }

  private async refreshVram(): Promise<void> {
    try {
      this.metrics = await invoke<RuntimeMetrics>("get_metrics");
      this.render();
      this.bindForm();
    } catch {
      // Non-fatal — VRAM section just stays stale
    }
  }

  private async applyHardwarePolicy(): Promise<void> {
    try {
      const response = await applyAndSeal("config-ai", {
          lora_mode: "dynamic",
          kv_cache_size: 32,
          tde_key: "hardware-policy-validation",
          accelerator: this.value("accelerator", "npu"),
          xdp_offload:
            (this.root.getElementById("xdp-offload") as HTMLInputElement | null)?.checked ?? true,
      });
      this.showFeedback(response.success ? "success" : "error", response.message);
    } catch (error) {
      this.showFeedback("error", error instanceof Error ? error.message : String(error));
    }
  }

  private value(id: string, fallback: string): string {
    const value = (this.root.getElementById(id) as HTMLSelectElement | null)?.value.trim();
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

customElements.define("tachyon-hardware-panel", TachyonHardwarePanel);

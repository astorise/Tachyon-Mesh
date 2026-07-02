import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { el } from "../../utils/dom-safe";
import { resilientInvoke as invoke } from "../../utils/network";
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

        <div id="feedback-zone" data-stagger-panel class="mt-4 rounded-xl border border-slate-800 bg-slate-900 px-4 py-3 font-mono text-xs text-slate-400">Model hardware strategy is configured from the AI orchestration panel.</div>
      </section>
    `);
    this.populateGpuBars();
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

    // Per-GPU bars are rendered as a placeholder; populated by
    // populateGpuBars() via the DOM API so user-controlled `gpu.model`
    // and `gpu.id` go through textContent and never innerHTML.
    const gpuBarsPlaceholder = gpus.length > 0
      ? `<div id="gpu-bars"></div>`
      : "";

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
        ${gpuBarsPlaceholder || clusterBar}
        ${offloadBadge}
      </div>
    `;
  }

  private populateGpuBars(): void {
    const host = this.root.getElementById("gpu-bars");
    if (!host) return;
    const vramPct = this.metrics?.vramUtilizationPct ?? null;
    const gpus = this.liveStatus?.gpus ?? [];
    host.replaceChildren(
      ...gpus.map((gpu) => {
        const used = gpu.vramUsedMb;
        const total = gpu.vramTotalMb;
        const pct = total > 0 ? Math.round((used / total) * 100) : vramPct ?? 0;
        const color = pct >= 90 ? "bg-red-500" : pct >= 80 ? "bg-amber-400" : "bg-purple-500";
        const label = total > 0 ? `${used} / ${total} MB` : `${pct}%`;

        const bar = el("div", { class: "w-full bg-slate-700 rounded-full h-2" },
          el("div", { class: `${color} h-2 rounded-full transition-all`, style: `width: ${pct}%` }),
        );

        const idSpan = el("span", { class: "text-slate-600" }, ` (${gpu.id})`);
        const header = el("div", { class: "flex justify-between text-xs text-slate-400 mb-1" },
          el("span", { class: "font-mono" }, gpu.model, " ", idSpan),
          el("span", { class: "font-mono" }, label),
        );

        return el("div", { class: "mb-3" }, header, bar);
      }),
    );
  }

  private bindForm(): void {
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

}

customElements.define("tachyon-hardware-panel", TachyonHardwarePanel);

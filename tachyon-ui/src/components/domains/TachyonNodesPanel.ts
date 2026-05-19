import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { resilientInvoke as invoke } from "../../utils/network";

type GpuStats = {
  id: string;
  model: string;
  vramTotalMb: number;
  vramUsedMb: number;
  computeUtilization: number;
};

type NodeCapabilities = {
  totalRamMb: number;
  availableRamMb: number;
  accelerators: string[];
  gpus: GpuStats[];
};

type EnrolledNode = {
  nodeId: string;
  status: string;
  lastSeen: number;
  capabilities: NodeCapabilities;
};

type ClusterHardwareSummary = {
  source: string;
  enrolledCount: number;
  onlineCount: number;
  staleCount: number;
  totalRamMb: number;
  gpuCount: number;
};

export class TachyonNodesPanel extends TachyonConfigDashboard {
  private nodes: EnrolledNode[] = [];
  private summary: ClusterHardwareSummary | null = null;
  private selected: EnrolledNode | null = null;
  private lastRefresh = 0;

  connectedCallback(): void {
    this.render();
    this.bindEvents();
    void this.load();
  }

  private async load(): Promise<void> {
    await this.withLoadingState(async () => {
      const [nodes, summary] = await Promise.all([
        invoke<EnrolledNode[]>("list_enrolled_nodes"),
        invoke<ClusterHardwareSummary>("get_cluster_hardware_summary"),
      ]);
      this.nodes = nodes;
      this.summary = summary;
      this.selected = nodes.find((node) => node.nodeId === this.selected?.nodeId) ?? null;
      this.render();
      this.bindEvents();
    });
  }

  private render(): void {
    const summary = this.summary ?? {
      source: "offline",
      enrolledCount: 0,
      onlineCount: 0,
      staleCount: 0,
      totalRamMb: 0,
      gpuCount: 0,
    };
    this.renderTemplate(`
      <section class="p-6 space-y-5 text-slate-300">
        <header class="flex flex-col gap-3 md:flex-row md:items-end md:justify-between">
          <div>
            <h2 class="text-2xl font-bold text-slate-100">Nodes</h2>
            <p class="text-xs font-mono text-slate-500">${summary.source}</p>
          </div>
          <button id="nodes-refresh" type="button" class="rounded border border-cyan-500/50 bg-cyan-500/10 px-3 py-2 text-xs font-semibold text-cyan-200 hover:bg-cyan-500/20">Refresh</button>
        </header>
        <div class="grid grid-cols-2 gap-3 lg:grid-cols-4">
          ${this.kpi("Enrolled", summary.enrolledCount)}
          ${this.kpi("Online", summary.onlineCount)}
          ${this.kpi("RAM MiB", summary.totalRamMb)}
          ${this.kpi("GPUs", summary.gpuCount)}
        </div>
        ${this.nodes.length === 0 ? this.emptyState() : this.table()}
        ${this.selected ? this.details(this.selected) : ""}
      </section>
    `);
  }

  private bindEvents(): void {
    this.root.getElementById("nodes-refresh")?.addEventListener("click", () => {
      const now = Date.now();
      if (now - this.lastRefresh < 1500) return;
      this.lastRefresh = now;
      void this.load();
    });
    this.root.querySelectorAll<HTMLButtonElement>("[data-node-id]").forEach((button) => {
      button.addEventListener("click", () => {
        this.selected = this.nodes.find((node) => node.nodeId === button.dataset.nodeId) ?? null;
        this.render();
        this.bindEvents();
      });
    });
  }

  private kpi(label: string, value: number): string {
    return `<article class="rounded-lg border border-slate-800 bg-slate-900 p-4"><p class="text-xs uppercase tracking-widest text-slate-500">${label}</p><strong class="mt-2 block font-mono text-2xl text-slate-100">${value}</strong></article>`;
  }

  private emptyState(): string {
    return `<article class="rounded-lg border border-slate-800 bg-slate-900 p-5 text-sm text-slate-400">No enrolled nodes yet. Generate an operator invite to enroll the first host.</article>`;
  }

  private table(): string {
    return `
      <article class="overflow-hidden rounded-lg border border-slate-800 bg-slate-900">
        <table class="w-full text-left text-sm">
          <thead class="bg-slate-950/60 text-xs uppercase tracking-widest text-slate-500">
            <tr><th class="p-3">Node</th><th class="p-3">Status</th><th class="p-3">RAM</th><th class="p-3">GPU</th><th class="p-3">Accelerators</th></tr>
          </thead>
          <tbody>
            ${this.nodes.map((node) => `
              <tr class="border-t border-slate-800">
                <td class="p-3"><button data-node-id="${this.escape(node.nodeId)}" class="font-mono text-cyan-300 hover:text-cyan-100">${this.escape(node.nodeId)}</button></td>
                <td class="p-3">${this.badge(node.status)}</td>
                <td class="p-3 font-mono">${node.capabilities.totalRamMb}</td>
                <td class="p-3 font-mono">${node.capabilities.gpus.length}</td>
                <td class="p-3">${this.escape(node.capabilities.accelerators.join(", ") || "cpu")}</td>
              </tr>`).join("")}
          </tbody>
        </table>
      </article>`;
  }

  private details(node: EnrolledNode): string {
    return `<aside class="rounded-lg border border-cyan-500/30 bg-slate-900 p-4">
      <h3 class="font-mono text-sm text-cyan-200">${this.escape(node.nodeId)}</h3>
      <div class="mt-3 grid gap-2 text-xs text-slate-400">
        ${node.capabilities.gpus.map((gpu) => `<div class="rounded border border-slate-800 p-2">${this.escape(gpu.id)} ${this.escape(gpu.model)} ${gpu.vramUsedMb}/${gpu.vramTotalMb} MiB</div>`).join("") || "<div>No GPU reported.</div>"}
      </div>
    </aside>`;
  }

  private badge(status: string): string {
    const color = status === "online" ? "text-cyan-200 border-cyan-500/40 bg-cyan-500/10" : "text-amber-200 border-amber-500/40 bg-amber-500/10";
    return `<span class="inline-flex rounded border px-2 py-1 text-xs ${color}">${this.escape(status)}</span>`;
  }

  private escape(value: string): string {
    return value.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
  }
}

customElements.define("tachyon-nodes-panel", TachyonNodesPanel);

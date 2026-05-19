import gsap from "gsap";

import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { resilientInvoke as invoke } from "../../utils/network";
import { t } from "../../utils/i18n";

type OverviewMetric = {
  key: "nodes" | "wasm" | "gpu";
  label: string;
  value: number;
  suffix: string;
  detail: string;
};

type MeshRouteSummary = {
  name: string;
  path: string;
  role: string;
  targetCount: number;
  requiresTee: boolean;
  encryptedVolumeCount: number;
};

type MeshGraphSnapshot = {
  source: string;
  status: string;
  routes: MeshRouteSummary[];
  batchTargets: string[];
};

type RuntimeMetrics = {
  source: string;
  errorRate: number;
  p50LatencyMs: number;
  p99LatencyMs: number;
  queueDepth: number;
};

type ClusterHardwareSummary = {
  enrolledCount: number;
};

export class TachyonOverviewPanel extends TachyonConfigDashboard {
  private metrics: OverviewMetric[] = this.initialMetrics();
  private status = t("overview.loading");
  private readonly onLanguageChanged = () => {
    this.metrics = this.translateMetrics(this.metrics);
    this.status = this.statusKeyForCurrentState();
    this.render(this.metrics, this.status);
    this.animateCounters();
  };

  async connectedCallback(): Promise<void> {
    window.addEventListener("i18n:language-changed", this.onLanguageChanged);
    this.metrics = this.initialMetrics();
    this.status = t("overview.loading");
    this.render(this.metrics, this.status);
    this.animateEntrance();
    await this.withLoadingState(async () => {
      const snapshot = await invoke<MeshGraphSnapshot>("get_mesh_graph");
      let runtime: RuntimeMetrics | null = null;
      try {
        runtime = await invoke<RuntimeMetrics>("get_metrics");
      } catch {
        runtime = null;
      }
      let hardware: ClusterHardwareSummary | null = null;
      try {
        hardware = await invoke<ClusterHardwareSummary>("get_cluster_hardware_summary");
      } catch {
        hardware = null;
      }
      this.metrics = this.metricsFromSources(snapshot, runtime, hardware);
      this.status = runtime?.source ? `${runtime.source}` : snapshot.status || t("overview.online");
      this.render(this.metrics, this.status);
      this.animateEntrance();
      this.animateCounters();
    });
  }

  disconnectedCallback(): void {
    window.removeEventListener("i18n:language-changed", this.onLanguageChanged);
  }

  private render(metrics: OverviewMetric[], status: string): void {
    this.renderTemplate(`
      <section class="p-6 space-y-8 text-slate-300">
        <header data-stagger-panel class="flex flex-col gap-3 md:flex-row md:items-end md:justify-between">
          <div>
            <h2 class="text-3xl font-light text-cyan-400">${t("overview.title.prefix")} <span class="font-bold text-slate-100">${t("overview.title.strong")}</span></h2>
            <p class="text-xs font-mono text-slate-500">${t("overview.telemetry")}</p>
          </div>
          <div class="rounded border border-cyan-500/30 bg-cyan-500/10 px-3 py-2 font-mono text-xs text-cyan-300">
            ${this.escapeStatus(status)}
          </div>
        </header>

        <div class="grid grid-cols-1 gap-6 lg:grid-cols-3">
          ${metrics
            .map(
              (metric) => `
                <article data-stagger-panel class="rounded-lg border border-slate-700 bg-slate-900 p-6 shadow-[0_0_24px_rgba(15,23,42,0.45)]">
                  <div class="mb-5 flex items-center justify-between">
                    <h3 class="text-sm font-semibold uppercase tracking-widest text-slate-400">${metric.label}</h3>
                    <span class="h-2 w-2 rounded-full bg-cyan-400 shadow-[0_0_12px_rgba(34,211,238,0.75)]"></span>
                  </div>
                  <div class="font-mono text-5xl font-light text-slate-100">
                    <span data-metric="${metric.key}" data-counter="${metric.value}" data-suffix="${metric.suffix}">0${metric.suffix}</span>
                  </div>
                  <p class="mt-4 min-h-12 text-sm leading-6 text-slate-500">${metric.detail}</p>
                </article>
              `,
            )
            .join("")}
        </div>

        <div data-stagger-panel class="grid grid-cols-1 gap-4 rounded-lg border border-slate-800 bg-slate-900/70 p-5 font-mono text-xs text-slate-400 md:grid-cols-3">
          <div><span class="text-cyan-400">${t("overview.routing")}</span> ${t("overview.routing.status")}</div>
          <div><span class="text-cyan-400">${t("overview.identity")}</span> ${t("overview.identity.status")}</div>
          <div><span class="text-cyan-400">${t("overview.observability")}</span> ${t("overview.observability.status")}</div>
        </div>
      </section>
    `);
  }

  private animateCounters(): void {
    this.root.querySelectorAll<HTMLElement>("[data-counter]").forEach((element) => {
      const target = Number(element.dataset.counter ?? "0");
      const suffix = element.dataset.suffix ?? "";
      const state = { value: 0 };
      void gsap.to(state, {
        value: target,
        duration: 1.2,
        ease: "power2.out",
        onUpdate: () => {
          element.textContent = `${Math.round(state.value)}${suffix}`;
        },
      });
    });
  }

  private metricsFromSources(snapshot: MeshGraphSnapshot, runtime: RuntimeMetrics | null, hardware: ClusterHardwareSummary | null): OverviewMetric[] {
    const sealedFallback = snapshot.routes.reduce((total, route) => total + Math.max(route.targetCount, 1), 0);
    const liveQueue = runtime?.queueDepth;
    const wasmInstances = typeof liveQueue === "number" && liveQueue > 0 ? liveQueue : sealedFallback;
    const gpuUtilization = this.gpuUtilizationFromMetrics(snapshot, runtime);
    const enrolledNodes = hardware?.enrolledCount ?? snapshot.batchTargets.length;
    return [
      { ...this.metricTemplate("nodes"), value: Math.max(enrolledNodes, 0) },
      { ...this.metricTemplate("wasm"), value: Math.round(wasmInstances) },
      { ...this.metricTemplate("gpu"), value: gpuUtilization },
    ];
  }

  private gpuUtilizationFromMetrics(snapshot: MeshGraphSnapshot, runtime: RuntimeMetrics | null): number {
    if (runtime) {
      const errorComponent = Math.min(100, Math.round((runtime.errorRate ?? 0) * 100));
      const latencyComponent = runtime.p99LatencyMs > 0
        ? Math.min(100, Math.round((runtime.p99LatencyMs / 500) * 100))
        : 0;
      return Math.max(errorComponent, latencyComponent);
    }
    if (snapshot.routes.length === 0) {
      return 0;
    }
    const acceleratedRoutes = snapshot.routes.filter((route) =>
      [route.name, route.path, route.role].some((value) => value.toLowerCase().includes("gpu") || value.toLowerCase().includes("ai")),
    ).length;
    return Math.min(100, Math.round(((acceleratedRoutes + snapshot.batchTargets.length) / Math.max(snapshot.routes.length, 1)) * 50));
  }

  private initialMetrics(): OverviewMetric[] {
    return [
      { ...this.metricTemplate("nodes"), value: 0 },
      { ...this.metricTemplate("wasm"), value: 0 },
      { ...this.metricTemplate("gpu"), value: 0 },
    ];
  }

  private translateMetrics(metrics: OverviewMetric[]): OverviewMetric[] {
    return metrics.map((metric) => ({ ...this.metricTemplate(metric.key), value: metric.value }));
  }

  private metricTemplate(key: OverviewMetric["key"]): Omit<OverviewMetric, "value"> {
    const suffix = key === "gpu" ? "%" : "";
    return {
      key,
      label: t(`overview.${key}.label`),
      suffix,
      detail: t(`overview.${key}.detail`),
    };
  }

  private statusKeyForCurrentState(): string {
    if (this.status === "Telemetry fetch failed" || this.status === "Échec de récupération de la télémétrie") {
      return t("overview.failed");
    }
    if (this.status === "Loading mesh telemetry..." || this.status === "Chargement de la télémétrie mesh...") {
      return t("overview.loading");
    }
    return this.status;
  }

  private escapeStatus(value: string): string {
    return value
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#039;");
  }
}

customElements.define("tachyon-overview-panel", TachyonOverviewPanel);

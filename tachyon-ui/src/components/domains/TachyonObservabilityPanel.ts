import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { applyAndSeal, resilientInvoke as invoke } from "../../utils/network";
import { t } from "../../utils/i18n";

type RuntimeMetrics = {
  source: string;
  errorRate: number;
  p50LatencyMs: number;
  p99LatencyMs: number;
  queueDepth: number;
};

type LogLine = {
  timestamp: string;
  level: string;
  target: string;
  message: string;
};

type ShadowDiff = {
  route: string;
  requestId: string;
  divergence: string;
  primaryStatus?: number;
  shadowStatus?: number;
};

export class TachyonObservabilityPanel extends TachyonConfigDashboard {
  private metrics: RuntimeMetrics | null = null;
  private logs: LogLine[] = [];
  private shadow: ShadowDiff[] = [];
  private readonly onLanguageChanged = () => this.render();

  async connectedCallback(): Promise<void> {
    window.addEventListener("i18n:language-changed", this.onLanguageChanged);
    this.render();
    this.bindEvents();
    this.animateEntrance();
    await this.refreshTelemetry();
  }

  disconnectedCallback(): void {
    window.removeEventListener("i18n:language-changed", this.onLanguageChanged);
  }

  private render(): void {
    this.renderTemplate(`
      <section class="p-6 space-y-6 text-slate-300">
        <header data-stagger-panel class="flex items-end justify-between gap-4 border-l-4 border-cyan-500 pl-4">
          <div>
            <h2 class="text-2xl font-bold text-slate-100">${t("observability.title")}</h2>
            <p class="text-sm font-mono text-slate-400">${t("observability.subtitle")}</p>
          </div>
          <button id="btn-refresh-observability" type="button" class="rounded-md border border-cyan-500/40 bg-cyan-500/10 px-3 py-2 text-xs font-medium text-cyan-200 hover:bg-cyan-500/20">${t("observability.refresh")}</button>
        </header>

        <article data-stagger-panel class="rounded-lg border border-slate-800 bg-slate-900 p-5">
          <h3 class="mb-3 text-sm font-semibold uppercase tracking-widest text-cyan-300">${t("observability.metrics.title")}</h3>
          ${this.renderMetrics()}
        </article>

        <article data-stagger-panel class="rounded-lg border border-slate-800 bg-slate-900 p-5">
          <h3 class="mb-3 text-sm font-semibold uppercase tracking-widest text-cyan-300">${t("observability.logs.title")}</h3>
          ${this.renderLogs()}
        </article>

        <article data-stagger-panel class="rounded-lg border border-slate-800 bg-slate-900 p-5">
          <h3 class="mb-3 text-sm font-semibold uppercase tracking-widest text-cyan-300">${t("observability.shadow.title")}</h3>
          ${this.renderShadow()}
        </article>

        <article data-stagger-panel class="rounded-lg border border-slate-700 bg-slate-800/40 p-6">
          <h3 class="mb-4 text-sm font-semibold uppercase tracking-widest text-cyan-300">${t("observability.config.title")}</h3>
          <form class="space-y-6">
            <label class="block text-xs uppercase tracking-widest text-cyan-500">${t("observability.config.endpoint")}
              <input id="otlp-endpoint" type="url" placeholder="https://otel-collector.tachyon.local/v1/traces" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm text-slate-200 outline-none transition-colors focus:border-cyan-400">
            </label>

            <label class="block text-xs uppercase tracking-widest text-cyan-500">${t("observability.config.log-level")}
              <select id="log-level" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm text-slate-200 outline-none transition-colors focus:border-cyan-400">
                <option value="debug">Debug</option>
                <option value="info" selected>Info</option>
                <option value="warn">Warn</option>
                <option value="error">Error</option>
              </select>
            </label>

            <button class="border border-cyan-500 px-6 py-3 font-bold text-cyan-500 transition-colors hover:bg-cyan-500 hover:text-slate-950">
              ${t("observability.config.update")}
            </button>
          </form>
        </article>

        <div id="feedback-zone" data-stagger-panel class="rounded-xl border border-slate-800 bg-slate-900 px-4 py-3 font-mono text-xs text-slate-400">${t("observability.feedback.empty")}</div>
      </section>
    `);
  }

  private bindEvents(): void {
    this.root.querySelector("form")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.applyObservabilityConfig();
    });
    this.root.getElementById("btn-refresh-observability")?.addEventListener("click", () => {
      void this.refreshTelemetry();
    });
  }

  private renderMetrics(): string {
    if (!this.metrics) {
      return `<p class="text-xs text-slate-500">${t("observability.metrics.empty")}</p>`;
    }
    const m = this.metrics;
    return `
      <dl class="grid grid-cols-2 gap-4 text-xs md:grid-cols-5">
        <div><dt class="text-slate-500">${t("observability.metrics.source")}</dt><dd class="font-mono text-cyan-300 break-all">${this.escape(m.source)}</dd></div>
        <div><dt class="text-slate-500">${t("observability.metrics.error-rate")}</dt><dd class="font-mono text-slate-200">${(m.errorRate * 100).toFixed(2)}%</dd></div>
        <div><dt class="text-slate-500">${t("observability.metrics.p50")}</dt><dd class="font-mono text-slate-200">${m.p50LatencyMs.toFixed(1)}</dd></div>
        <div><dt class="text-slate-500">${t("observability.metrics.p99")}</dt><dd class="font-mono text-slate-200">${m.p99LatencyMs.toFixed(1)}</dd></div>
        <div><dt class="text-slate-500">${t("observability.metrics.queue")}</dt><dd class="font-mono text-slate-200">${m.queueDepth}</dd></div>
      </dl>
    `;
  }

  private renderLogs(): string {
    if (this.logs.length === 0) {
      return `<p class="text-xs text-slate-500">${t("observability.logs.empty")}</p>`;
    }
    return `
      <ul class="max-h-64 space-y-1 overflow-y-auto font-mono text-xs">
        ${this.logs
          .map(
            (line) => `<li class="border-b border-slate-800 py-1"><span class="text-slate-500">${this.escape(line.timestamp)}</span> <span class="text-cyan-300">[${this.escape(line.level)}]</span> <span class="text-slate-400">${this.escape(line.target)}</span> ${this.escape(line.message)}</li>`,
          )
          .join("")}
      </ul>
    `;
  }

  private renderShadow(): string {
    if (this.shadow.length === 0) {
      return `<p class="text-xs text-slate-500">${t("observability.shadow.empty")}</p>`;
    }
    return `
      <ul class="space-y-2 font-mono text-xs">
        ${this.shadow
          .map((diff) => {
            const primary = diff.primaryStatus !== undefined ? diff.primaryStatus : "—";
            const shadow = diff.shadowStatus !== undefined ? diff.shadowStatus : "—";
            return `<li class="rounded border border-slate-800 bg-slate-950/50 px-3 py-2"><div class="flex justify-between"><span class="text-cyan-300">${this.escape(diff.route)}</span><span class="text-slate-500">${this.escape(diff.requestId)}</span></div><div class="text-slate-400">${this.escape(diff.divergence)} (primary=${primary}, shadow=${shadow})</div></li>`;
          })
          .join("")}
      </ul>
    `;
  }

  private async refreshTelemetry(): Promise<void> {
    try {
      this.metrics = await invoke<RuntimeMetrics>("get_metrics");
    } catch {
      this.metrics = null;
    }
    try {
      this.logs = await invoke<LogLine[]>("tail_logs", { lines: 50 });
    } catch {
      this.logs = [];
    }
    try {
      this.shadow = await invoke<ShadowDiff[]>("get_shadow_diffs");
    } catch {
      this.shadow = [];
    }
    this.render();
    this.bindEvents();
    this.animateEntrance();
  }

  private async applyObservabilityConfig(): Promise<void> {
    try {
      const response = await applyAndSeal("observability", {
          otlp_endpoint: this.value("otlp-endpoint", ""),
          log_level: this.value("log-level", "info"),
      });
      this.showFeedback(response.success ? "success" : "error", response.message);
    } catch (error) {
      this.showFeedback("error", error instanceof Error ? error.message : String(error));
    }
  }

  private value(id: string, fallback: string): string {
    const value = (this.root.getElementById(id) as HTMLInputElement | HTMLSelectElement | null)?.value.trim();
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

customElements.define("tachyon-observability-panel", TachyonObservabilityPanel);

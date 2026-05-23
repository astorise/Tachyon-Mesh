import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { el } from "../../utils/dom-safe";
import { applyAndSeal, resilientInvoke as invoke } from "../../utils/network";
import { t } from "../../utils/i18n";
import { readScopeMetrics } from "../../controllers/scopesController";

type RuntimeMetrics = {
  source: string;
  errorRate: number;
  p50LatencyMs: number;
  p99LatencyMs: number;
  queueDepth: number;
  scopeDenialTotal?: number;
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
  private scopeDenialTotal = 0;
  private scopeRefreshInterval: ReturnType<typeof setInterval> | null = null;
  private readonly onLanguageChanged = () => this.render();

  async connectedCallback(): Promise<void> {
    window.addEventListener("i18n:language-changed", this.onLanguageChanged);
    this.render();
    this.bindEvents();
    this.animateEntrance();
    await this.refreshTelemetry();
    await this.refreshScopeDenials();
    this.scopeRefreshInterval = setInterval(() => { void this.refreshScopeDenials(); }, 30_000);
  }

  disconnectedCallback(): void {
    window.removeEventListener("i18n:language-changed", this.onLanguageChanged);
    if (this.scopeRefreshInterval !== null) {
      clearInterval(this.scopeRefreshInterval);
      this.scopeRefreshInterval = null;
    }
  }

  private async refreshScopeDenials(): Promise<void> {
    try {
      const result = await readScopeMetrics();
      this.scopeDenialTotal = result.scopeDenialTotal;
      this.updateScopeDenialsWidget();
    } catch {
      // Non-fatal: widget stays at last known value.
    }
  }

  private updateScopeDenialsWidget(): void {
    const el = this.root.getElementById("scope-denial-count");
    if (el) el.textContent = String(this.scopeDenialTotal);
    const empty = this.root.getElementById("scope-denial-empty");
    const counter = this.root.getElementById("scope-denial-counter");
    if (this.scopeDenialTotal === 0) {
      empty?.classList.remove("hidden");
      counter?.classList.add("hidden");
    } else {
      empty?.classList.add("hidden");
      counter?.classList.remove("hidden");
    }
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

        <article data-stagger-panel class="rounded-lg border border-slate-800 bg-slate-900 p-5">
          <h3 class="mb-3 text-sm font-semibold uppercase tracking-widest text-cyan-300">Scope Denials</h3>
          <div id="scope-denial-empty" class="${this.scopeDenialTotal === 0 ? "" : "hidden"}">
            <p class="text-xs text-emerald-400">No scope denials recorded — scopes are working correctly.</p>
          </div>
          <div id="scope-denial-counter" class="${this.scopeDenialTotal !== 0 ? "" : "hidden"}">
            <p class="text-xs text-slate-400 mb-1">lifetime denials, all categories and deployments:</p>
            <p class="font-mono text-2xl font-bold text-amber-400" id="scope-denial-count">${this.scopeDenialTotal}</p>
            <p class="text-[10px] text-slate-500 mt-1">Per-category breakdown: <span class="font-mono">faas_scope_denials_total{"{deployment,category}"}</span> in prometheus.</p>
            <a href="#" id="link-configure-scopes" class="text-[10px] text-cyan-400 hover:underline mt-1 inline-block">Configure scopes →</a>
          </div>
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
    this.populateObservability();
  }

  private bindEvents(): void {
    this.root.querySelector("form")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.applyObservabilityConfig();
    });
    this.root.getElementById("btn-refresh-observability")?.addEventListener("click", () => {
      void this.refreshTelemetry();
    });
    this.root.getElementById("link-configure-scopes")?.addEventListener("click", (e) => {
      e.preventDefault();
      window.dispatchEvent(new CustomEvent("app:navigate", { detail: { panel: "routing" } }));
    });
  }

  private renderMetrics(): string {
    if (!this.metrics) {
      return `<p class="text-xs text-slate-500">${t("observability.metrics.empty")}</p>`;
    }
    // Static structure — source field populated via DOM API.
    return `
      <dl class="grid grid-cols-2 gap-4 text-xs md:grid-cols-5">
        <div><dt class="text-slate-500">${t("observability.metrics.source")}</dt><dd id="metrics-source" class="font-mono text-cyan-300 break-all"></dd></div>
        <div><dt class="text-slate-500">${t("observability.metrics.error-rate")}</dt><dd class="font-mono text-slate-200">${(this.metrics.errorRate * 100).toFixed(2)}%</dd></div>
        <div><dt class="text-slate-500">${t("observability.metrics.p50")}</dt><dd class="font-mono text-slate-200">${this.metrics.p50LatencyMs.toFixed(1)}</dd></div>
        <div><dt class="text-slate-500">${t("observability.metrics.p99")}</dt><dd class="font-mono text-slate-200">${this.metrics.p99LatencyMs.toFixed(1)}</dd></div>
        <div><dt class="text-slate-500">${t("observability.metrics.queue")}</dt><dd class="font-mono text-slate-200">${this.metrics.queueDepth}</dd></div>
      </dl>
    `;
  }

  private renderLogs(): string {
    if (this.logs.length === 0) {
      return `<p class="text-xs text-slate-500">${t("observability.logs.empty")}</p>`;
    }
    return `<ul id="observability-logs" class="max-h-64 space-y-1 overflow-y-auto font-mono text-xs"></ul>`;
  }

  private renderShadow(): string {
    if (this.shadow.length === 0) {
      return `<p class="text-xs text-slate-500">${t("observability.shadow.empty")}</p>`;
    }
    return `<ul id="observability-shadow" class="space-y-2 font-mono text-xs"></ul>`;
  }

  private populateObservability(): void {
    // Metrics source (user data)
    if (this.metrics) {
      const sourceEl = this.root.getElementById("metrics-source");
      if (sourceEl) sourceEl.textContent = this.metrics.source;
    }

    // Log lines
    const logsHost = this.root.getElementById("observability-logs");
    if (logsHost) {
      logsHost.replaceChildren(
        ...this.logs.map((line) =>
          el("li", { class: "border-b border-slate-800 py-1" },
            el("span", { class: "text-slate-500" }, line.timestamp),
            " ",
            el("span", { class: "text-cyan-300" }, `[${line.level}]`),
            " ",
            el("span", { class: "text-slate-400" }, line.target),
            " ",
            line.message,
          ),
        ),
      );
    }

    // Shadow diffs
    const shadowHost = this.root.getElementById("observability-shadow");
    if (shadowHost) {
      shadowHost.replaceChildren(
        ...this.shadow.map((diff) => {
          const primary = diff.primaryStatus !== undefined ? String(diff.primaryStatus) : "—";
          const shadow = diff.shadowStatus !== undefined ? String(diff.shadowStatus) : "—";
          return el("li", { class: "rounded border border-slate-800 bg-slate-950/50 px-3 py-2" },
            el("div", { class: "flex justify-between" },
              el("span", { class: "text-cyan-300" }, diff.route),
              el("span", { class: "text-slate-500" }, diff.requestId),
            ),
            el("div", { class: "text-slate-400" },
              diff.divergence, ` (primary=${primary}, shadow=${shadow})`,
            ),
          );
        }),
      );
    }
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
}

customElements.define("tachyon-observability-panel", TachyonObservabilityPanel);

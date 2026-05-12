import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { applyAndSeal, resilientInvoke } from "../../utils/network";
import { t } from "../../utils/i18n";

type CanaryStatusEntry = {
  routePath: string;
  currentVersion: string;
  nextVersion: string;
  weightPct: number;
  phase: string;
  nextReqCount: number;
  nextErrCount: number;
};

export class TachyonWorkloadsPanel extends TachyonConfigDashboard {
  private rollouts: CanaryStatusEntry[] = [];
  private readonly onLanguageChanged = () => { this.render(); this.bindForm(); };

  async connectedCallback(): Promise<void> {
    window.addEventListener("i18n:language-changed", this.onLanguageChanged);
    this.render();
    this.bindForm();
    this.animateEntrance();
    await this.refreshRollouts();
  }

  disconnectedCallback(): void {
    window.removeEventListener("i18n:language-changed", this.onLanguageChanged);
  }

  private async refreshRollouts(): Promise<void> {
    try {
      this.rollouts = await resilientInvoke<CanaryStatusEntry[]>("fetch_canary_status");
    } catch {
      this.rollouts = [];
    }
    this.render();
    this.bindForm();
  }

  private render(): void {
    this.renderTemplate(`
      <section class="p-6 space-y-6 text-slate-300">
        <header data-stagger-panel class="flex flex-col gap-3 border-b border-slate-700 pb-4 md:flex-row md:items-center md:justify-between">
          <div>
            <h2 class="text-2xl font-bold text-slate-100">${t("workloads.title")}</h2>
            <p class="text-sm font-mono text-slate-400">${t("workloads.subtitle")}</p>
          </div>
          <span class="w-fit rounded border border-cyan-500/30 bg-cyan-900/50 px-2 py-1 text-[10px] text-cyan-400">${t("workloads.badge")}</span>
        </header>

        <form class="grid grid-cols-1 gap-6 rounded-lg border border-slate-700 bg-slate-800/40 p-6 md:grid-cols-2">
          <label data-stagger-panel class="block text-xs uppercase tracking-widest text-slate-400">${t("workloads.field.engine")}
            <select id="engine" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm text-slate-200 outline-none transition-colors focus:border-cyan-400">
              <option value="wasmtime">${t("workloads.option.wasmtime")}</option>
              <option value="smolvm">${t("workloads.option.smolvm")}</option>
              <option value="legacy">${t("workloads.option.container")}</option>
            </select>
          </label>

          <label data-stagger-panel class="block text-xs uppercase tracking-widest text-slate-400">${t("workloads.field.secret")}
            <input id="secret-ref" type="text" placeholder="${t("workloads.placeholder.secret")}" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm text-slate-200 outline-none transition-colors focus:border-cyan-400">
          </label>

          <label data-stagger-panel class="block text-xs uppercase tracking-widest text-slate-400 md:col-span-2">${t("workloads.field.strategy")}
            <select id="strategy" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm text-slate-200 outline-none transition-colors focus:border-cyan-400">
              <option value="rolling">${t("workloads.option.rolling")}</option>
              <option value="canary">${t("workloads.option.canary")}</option>
            </select>
          </label>

          <div id="canary-config-fields" class="md:col-span-2 grid grid-cols-1 gap-4 hidden rounded-lg border border-amber-500/30 bg-amber-500/5 p-4 md:grid-cols-2">
            <p class="md:col-span-2 text-[11px] uppercase tracking-widest text-amber-400">${t("workloads.canary.section")}</p>

            <label class="block text-xs uppercase tracking-widest text-slate-400">${t("workloads.canary.nextVersion")}
              <input id="canary-next-version" type="text" placeholder="${t("workloads.canary.nextVersionPlaceholder")}" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm text-slate-200 outline-none transition-colors focus:border-cyan-400">
            </label>

            <label class="block text-xs uppercase tracking-widest text-slate-400">${t("workloads.canary.stepWeight")}
              <input id="canary-step-weight" type="number" min="1" max="100" value="10" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm text-slate-200 outline-none transition-colors focus:border-cyan-400">
            </label>

            <label class="block text-xs uppercase tracking-widest text-slate-400">${t("workloads.canary.intervalSecs")}
              <input id="canary-interval-secs" type="number" min="10" value="60" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm text-slate-200 outline-none transition-colors focus:border-cyan-400">
            </label>

            <label class="block text-xs uppercase tracking-widest text-slate-400">${t("workloads.canary.maxErrorRate")}
              <input id="canary-max-error-rate" type="number" min="0" max="1" step="0.01" value="0.05" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm text-slate-200 outline-none transition-colors focus:border-cyan-400">
            </label>
          </div>

          <button data-stagger-panel class="border border-cyan-500 px-6 py-3 font-bold text-cyan-500 transition-colors hover:bg-cyan-500 hover:text-slate-950 md:col-span-2 md:w-fit">
            ${t("workloads.button")}
          </button>
        </form>

        ${this.renderRolloutStatus()}

        <div id="feedback-zone" data-stagger-panel class="rounded-xl border border-slate-800 bg-slate-900 px-4 py-3 font-mono text-xs text-slate-400">${t("workloads.feedback.empty")}</div>
      </section>
    `);
  }

  private renderRolloutStatus(): string {
    const activeRollouts = this.rollouts.filter((r) => r.phase === "stepping");
    if (activeRollouts.length === 0 && this.rollouts.length === 0) {
      return "";
    }

    const entries = this.rollouts.map((r) => {
      const phaseBadge = this.phaseBadge(r.phase);
      const errorRate = r.nextReqCount > 0
        ? ((r.nextErrCount / r.nextReqCount) * 100).toFixed(1)
        : "0.0";
      const isActive = r.phase === "stepping";
      return `
        <div class="rounded-lg border ${isActive ? "border-amber-500/40 bg-amber-500/5" : "border-slate-700 bg-slate-900"} p-4 space-y-3">
          <div class="flex items-center justify-between gap-3">
            <div>
              <span class="font-mono text-sm text-cyan-300">${this.escape(r.routePath)}</span>
              <span class="ml-2 text-[10px] text-slate-500">${this.escape(r.currentVersion)} → ${this.escape(r.nextVersion)}</span>
            </div>
            <div class="flex items-center gap-2">
              ${phaseBadge}
              ${isActive ? `<button data-abort-route="${this.escape(r.routePath)}" class="rounded border border-red-500/40 bg-red-500/10 px-2 py-1 text-[11px] text-red-300 hover:bg-red-500/20">${t("workloads.canary.abort")}</button>` : ""}
            </div>
          </div>
          ${isActive ? `
          <div>
            <div class="mb-1 flex justify-between text-[11px] text-slate-400">
              <span>${t("workloads.canary.traffic")}: ${r.weightPct}%</span>
              <span>${t("workloads.canary.errorRate")}: ${errorRate}%</span>
            </div>
            <div class="h-2 w-full overflow-hidden rounded-full bg-slate-700">
              <div class="h-full rounded-full bg-amber-400 transition-all" style="width:${r.weightPct}%"></div>
            </div>
          </div>` : ""}
        </div>
      `;
    }).join("");

    return `
      <article data-stagger-panel class="space-y-3">
        <div class="flex items-center justify-between">
          <h3 class="text-sm font-semibold uppercase tracking-widest text-amber-400">${t("workloads.canary.statusTitle")}</h3>
          <button id="btn-refresh-rollouts" type="button" class="rounded border border-slate-700 bg-slate-800 px-2 py-1 text-[11px] text-slate-300 hover:bg-slate-700">${t("workloads.canary.refresh")}</button>
        </div>
        ${entries || `<p class="text-xs text-slate-500">${t("workloads.canary.noRollouts")}</p>`}
      </article>
    `;
  }

  private phaseBadge(phase: string): string {
    if (phase === "stepping") {
      return `<span class="rounded bg-amber-500/20 px-2 py-0.5 text-[10px] text-amber-300">${t("workloads.canary.phase.stepping")}</span>`;
    }
    if (phase === "promoted") {
      return `<span class="rounded bg-emerald-500/20 px-2 py-0.5 text-[10px] text-emerald-300">${t("workloads.canary.phase.promoted")}</span>`;
    }
    return `<span class="rounded bg-red-500/20 px-2 py-0.5 text-[10px] text-red-300">${t("workloads.canary.phase.rolledBack")}</span>`;
  }

  private escape(value: string): string {
    return value
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#039;");
  }

  private bindForm(): void {
    const strategySelect = this.root.getElementById("strategy") as HTMLSelectElement | null;
    const canaryFields = this.root.getElementById("canary-config-fields");
    if (strategySelect && canaryFields) {
      const toggle = () => {
        canaryFields.classList.toggle("hidden", strategySelect.value !== "canary");
      };
      toggle();
      strategySelect.addEventListener("change", toggle);
    }

    this.root.querySelector("form")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.applyWorkloadContract();
    });

    this.root.getElementById("btn-refresh-rollouts")?.addEventListener("click", () => {
      void this.refreshRollouts();
    });

    this.root.querySelectorAll<HTMLButtonElement>("button[data-abort-route]").forEach((btn) => {
      btn.addEventListener("click", () => {
        const routePath = btn.dataset.abortRoute ?? "";
        if (!routePath) return;
        if (!window.confirm(t("workloads.canary.confirmAbort").replace("{route}", routePath))) return;
        void this.abortRollout(routePath);
      });
    });
  }

  private async abortRollout(routePath: string): Promise<void> {
    try {
      await resilientInvoke("abort_canary_rollout", { routePath });
      this.showFeedback("success", t("workloads.canary.aborted").replace("{route}", routePath));
      await this.refreshRollouts();
    } catch (error) {
      this.showFeedback("error", error instanceof Error ? error.message : String(error));
    }
  }

  private async applyWorkloadContract(): Promise<void> {
    const strategy = this.value("strategy", "rolling");
    const payload: Record<string, unknown> = {
      engine: this.value("engine", "wasmtime"),
      secret_ref: this.value("secret-ref", ""),
      strategy,
    };

    if (strategy === "canary") {
      payload.canary = {
        next_version: this.value("canary-next-version", ""),
        step_weight: Number(this.value("canary-step-weight", "10")),
        interval_secs: Number(this.value("canary-interval-secs", "60")),
        max_error_rate: Number(this.value("canary-max-error-rate", "0.05")),
      };
    }

    try {
      const response = await applyAndSeal("workloads", payload);
      this.showFeedback(response.success ? "success" : "error", response.message);
      if (response.success) {
        await this.refreshRollouts();
      }
    } catch (error) {
      this.showFeedback("error", error instanceof Error ? error.message : String(error));
    }
  }

  private value(id: string, fallback: string): string {
    const value = (
      this.root.getElementById(id) as HTMLInputElement | HTMLSelectElement | null
    )?.value.trim();
    return value ? value : fallback;
  }
}

customElements.define("tachyon-workloads-panel", TachyonWorkloadsPanel);

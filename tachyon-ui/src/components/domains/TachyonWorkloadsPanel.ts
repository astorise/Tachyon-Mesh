import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { el } from "../../utils/dom-safe";
import { resilientInvoke } from "../../utils/network";
import { t } from "../../utils/i18n";
import { invoke } from "@tauri-apps/api/core";
import { listManifestRoutes, writeRouteCanary, type ManifestRouteOption } from "../../controllers/manifestConfigController";

type ImportPackageResult = {
  importedModules: Array<{ name: string; assetUri: string }>;
  skippedModules: string[];
  routesAdded: number;
};

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
  private routes: ManifestRouteOption[] = [];
  private importFile: File | null = null;
  private readonly onLanguageChanged = () => { this.render(); this.bindForm(); };

  async connectedCallback(): Promise<void> {
    window.addEventListener("i18n:language-changed", this.onLanguageChanged);
    this.render();
    this.bindForm();
    this.animateEntrance();
    await this.refreshManifestRoutes();
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

  private async refreshManifestRoutes(): Promise<void> {
    try {
      this.routes = await listManifestRoutes();
    } catch {
      this.routes = [];
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

            <label class="block text-xs uppercase tracking-widest text-slate-400 md:col-span-2">${t("workloads.canary.route")}
              <select id="canary-route" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm text-slate-200 outline-none transition-colors focus:border-cyan-400">
                ${this.renderRouteOptions()}
              </select>
            </label>

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

        <section data-stagger-panel class="rounded-lg border border-slate-700 bg-slate-800/40 p-6 space-y-4">
          <div>
            <h3 class="text-sm font-semibold uppercase tracking-widest text-cyan-400">${t("workloads.import.title")}</h3>
            <p class="mt-1 text-xs text-slate-400">${t("workloads.import.description")}</p>
          </div>
          <div class="flex flex-col gap-3 sm:flex-row sm:items-center">
            <label class="flex cursor-pointer items-center gap-2 rounded border border-slate-600 bg-slate-900 px-4 py-2 text-sm text-slate-200 hover:border-cyan-500 transition-colors">
              ${t("workloads.import.button")}
              <input id="import-pkg-file" type="file" accept=".tar.gz,.tgz" class="sr-only">
            </label>
            <span id="import-pkg-name" class="font-mono text-xs text-slate-500">${t("workloads.import.noFile")}</span>
            <button id="import-pkg-btn" type="button" disabled
              class="rounded border border-cyan-500/40 bg-cyan-900/30 px-5 py-2 text-sm font-bold text-cyan-400 transition-colors hover:bg-cyan-500 hover:text-slate-950 disabled:cursor-not-allowed disabled:opacity-40">
              ${t("workloads.import.deploy")}
            </button>
          </div>
        </section>

        <div id="feedback-zone" data-stagger-panel class="rounded-xl border border-slate-800 bg-slate-900 px-4 py-3 font-mono text-xs text-slate-400">${t("workloads.feedback.empty")}</div>
      </section>
    `);
  }

  private renderRolloutStatus(): string {
    const activeRollouts = this.rollouts.filter((r) => r.phase === "stepping");
    if (activeRollouts.length === 0 && this.rollouts.length === 0) {
      return "";
    }
    // Static structure — entries populated via DOM API in populateRollouts().
    return `
      <article data-stagger-panel class="space-y-3">
        <div class="flex items-center justify-between">
          <h3 class="text-sm font-semibold uppercase tracking-widest text-amber-400">${t("workloads.canary.statusTitle")}</h3>
          <button id="btn-refresh-rollouts" type="button" class="rounded border border-slate-700 bg-slate-800 px-2 py-1 text-[11px] text-slate-300 hover:bg-slate-700">${t("workloads.canary.refresh")}</button>
        </div>
        <div id="rollouts-list"></div>
      </article>
    `;
  }

  private renderRouteOptions(): string {
    if (this.routes.length === 0) {
      return `<option value="">${t("workloads.canary.noRoutes")}</option>`;
    }
    return this.routes
      .map((route) => {
        const label = [route.path, route.name, route.version].filter(Boolean).join(" - ");
        return `<option value="${this.escape(route.path)}">${this.escape(label)}</option>`;
      })
      .join("");
  }

  private populateRollouts(): void {
    const host = this.root.getElementById("rollouts-list");
    if (!host) return;

    if (this.rollouts.length === 0) {
      host.replaceChildren(el("p", { class: "text-xs text-slate-500" }, t("workloads.canary.noRollouts")));
      return;
    }

    host.replaceChildren(
      ...this.rollouts.map((r) => {
        const errorRate = r.nextReqCount > 0
          ? ((r.nextErrCount / r.nextReqCount) * 100).toFixed(1)
          : "0.0";
        const isActive = r.phase === "stepping";
        const containerClass = "rounded-lg border " +
          (isActive ? "border-amber-500/40 bg-amber-500/5" : "border-slate-700 bg-slate-900") +
          " p-4 space-y-3";

        const phaseBadgeNode = this.phaseBadgeNode(r.phase);

        const headerRight = el("div", { class: "flex items-center gap-2" }, phaseBadgeNode);
        if (isActive) {
          headerRight.appendChild(
            el("button", {
              "data-abort-route": r.routePath,
              class: "rounded border border-red-500/40 bg-red-500/10 px-2 py-1 text-[11px] text-red-300 hover:bg-red-500/20",
            }, t("workloads.canary.abort")),
          );
        }

        const header = el("div", { class: "flex items-center justify-between gap-3" },
          el("div", {},
            el("span", { class: "font-mono text-sm text-cyan-300" }, r.routePath),
            el("span", { class: "ml-2 text-[10px] text-slate-500" }, `${r.currentVersion} → ${r.nextVersion}`),
          ),
          headerRight,
        );

        const body = el("div", { class: containerClass }, header);
        if (isActive) {
          body.appendChild(
            el("div", {},
              el("div", { class: "mb-1 flex justify-between text-[11px] text-slate-400" },
                el("span", {}, `${t("workloads.canary.traffic")}: ${r.weightPct}%`),
                el("span", {}, `${t("workloads.canary.errorRate")}: ${errorRate}%`),
              ),
              el("div", { class: "h-2 w-full overflow-hidden rounded-full bg-slate-700" },
                el("div", { class: "h-full rounded-full bg-amber-400 transition-all", style: `width:${r.weightPct}%` }),
              ),
            ),
          );
        }
        return body;
      }),
    );
  }

  private phaseBadgeNode(phase: string): HTMLElement {
    if (phase === "stepping") {
      return el("span", { class: "rounded bg-amber-500/20 px-2 py-0.5 text-[10px] text-amber-300" }, t("workloads.canary.phase.stepping"));
    }
    if (phase === "promoted") {
      return el("span", { class: "rounded bg-emerald-500/20 px-2 py-0.5 text-[10px] text-emerald-300" }, t("workloads.canary.phase.promoted"));
    }
    return el("span", { class: "rounded bg-red-500/20 px-2 py-0.5 text-[10px] text-red-300" }, t("workloads.canary.phase.rolledBack"));
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

    const fileInput = this.root.getElementById("import-pkg-file") as HTMLInputElement | null;
    const fileLabel = this.root.getElementById("import-pkg-name");
    const importBtn = this.root.getElementById("import-pkg-btn") as HTMLButtonElement | null;

    fileInput?.addEventListener("change", () => {
      this.importFile = fileInput.files?.[0] ?? null;
      if (fileLabel) {
        fileLabel.textContent = this.importFile?.name ?? t("workloads.import.noFile");
      }
      if (importBtn) {
        importBtn.disabled = !this.importFile;
      }
    });

    importBtn?.addEventListener("click", () => {
      void this.importPackage();
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

  private async importPackage(): Promise<void> {
    if (!this.importFile) return;
    const importBtn = this.root.getElementById("import-pkg-btn") as HTMLButtonElement | null;
    if (importBtn) importBtn.disabled = true;
    try {
      const buffer = await this.importFile.arrayBuffer();
      const bytes = Array.from(new Uint8Array(buffer));
      const result = await invoke<ImportPackageResult>("import_faas_package", { bytes });
      if (result.routesAdded === 0) {
        this.showFeedback("error", t("workloads.import.empty"));
      } else {
        const skippedText = result.skippedModules.length > 0
          ? result.skippedModules.join(", ")
          : "none";
        this.showFeedback(
          "success",
          t("workloads.import.success")
            .replace("{count}", String(result.routesAdded))
            .replace("{skipped}", skippedText),
        );
      }
    } catch (error) {
      this.showFeedback("error", error instanceof Error ? error.message : String(error));
    } finally {
      if (importBtn) importBtn.disabled = false;
    }
  }

  private async applyWorkloadContract(): Promise<void> {
    const strategy = this.value("strategy", "rolling");
    if (strategy !== "canary") {
      this.showFeedback("error", t("workloads.feedback.manifestOnly"));
      return;
    }

    try {
      const response = await writeRouteCanary(this.value("canary-route", ""), {
        next_version: this.value("canary-next-version", ""),
        step_weight: Number(this.value("canary-step-weight", "10")),
        interval_secs: Number(this.value("canary-interval-secs", "60")),
        max_error_rate: Number(this.value("canary-max-error-rate", "0.05")),
      });
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

  private escape(value: string): string {
    return value
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }
}

customElements.define("tachyon-workloads-panel", TachyonWorkloadsPanel);

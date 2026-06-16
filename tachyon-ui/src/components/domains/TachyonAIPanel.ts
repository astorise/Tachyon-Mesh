import gsap from "gsap";

import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { applyAndSeal, resilientInvoke as invoke } from "../../utils/network";
import { t } from "../../utils/i18n";
import "./TachyonModelUploadPanel";

type RuntimeMetrics = {
  source: string;
  errorRate: number;
  p50LatencyMs: number;
  p99LatencyMs: number;
  queueDepth: number;
  vramUtilizationPct: number;
  ramOffloadActive: boolean;
};

type AvailableAiModel = {
  id: string;
  alias: string;
  engine: string;
};

type ModelLoadMode = "integrity" | "layer" | "layer-batch";

export class TachyonAIPanel extends TachyonConfigDashboard {
  private metrics: RuntimeMetrics | null = null;
  private models: AvailableAiModel[] = [];
  private modelLoadModes: Record<string, ModelLoadMode> = {};
  private readonly onLanguageChanged = () => { this.render(); this.bindEvents(); };
  // The child `<tachyon-model-upload-panel>` fires this (bubbling/composed) on a
  // successful upload; re-fetch the model list so the new model appears without a
  // manual refresh.
  private readonly onModelUploaded = () => { void this.refreshAiState(); };

  async connectedCallback(): Promise<void> {
    window.addEventListener("i18n:language-changed", this.onLanguageChanged);
    this.addEventListener("tachyon:model-uploaded", this.onModelUploaded);
    this.render();
    this.bindEvents();
    this.animateGlitchEntrance();
    await this.refreshAiState();
  }

  disconnectedCallback(): void {
    window.removeEventListener("i18n:language-changed", this.onLanguageChanged);
    this.removeEventListener("tachyon:model-uploaded", this.onModelUploaded);
  }

  private async refreshAiState(): Promise<void> {
    const metrics = await invoke<RuntimeMetrics>("get_metrics").catch(() => null);
    // Unlike metrics (best-effort), a failed model-list fetch is surfaced: silently
    // swallowing it (`catch(() => [])`) is exactly what made an uploaded-but-
    // unlistable model look like "nothing happened". Keep the previous list on error.
    let modelsError: string | null = null;
    try {
      this.models = await invoke<AvailableAiModel[]>("list_available_models");
    } catch (error) {
      modelsError = error instanceof Error ? error.message : String(error);
    }
    this.metrics = metrics;
    this.reconcileModelLoadModes();
    this.render();
    this.bindEvents();
    if (modelsError) {
      this.showFeedback("error", t("ai.model.loadError").replace("{message}", modelsError));
    }
  }

  private render(): void {
    this.renderTemplate(`
      <section class="p-6 space-y-8 text-slate-300">
        <header data-stagger-panel class="flex flex-col gap-4 md:flex-row md:items-end md:justify-between">
          <div>
            <h2 class="text-3xl font-light text-cyan-400">${t("ai.title")} <span class="font-bold text-slate-100">${t("ai.title.strong")}</span></h2>
            <p class="text-xs font-mono text-slate-500">${t("ai.subtitle")}</p>
          </div>
          <div class="text-left md:text-right">
            <span class="block text-[10px] uppercase text-cyan-500/70">${t("ai.status.label")}</span>
            <span class="font-mono text-xs text-emerald-400">${t("ai.status.value")}</span>
          </div>
        </header>

        ${this.renderVramMetrics()}

        <tachyon-model-upload-panel data-stagger-panel></tachyon-model-upload-panel>

        ${this.renderModelLoadModes()}

        <form class="space-y-6">
          <div class="grid grid-cols-1 gap-6 lg:grid-cols-3">
            <div data-stagger-panel class="space-y-4 rounded border border-slate-700 bg-slate-800/70 p-5">
              <label class="block text-sm font-bold uppercase tracking-widest text-slate-300">${t("ai.field.lora")}
                <select id="lora-mode" class="mt-3 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm text-cyan-300 outline-none transition-colors focus:border-cyan-400">
                  <option value="dynamic">${t("ai.option.lora.dynamic")}</option>
                  <option value="static">${t("ai.option.lora.static")}</option>
                </select>
              </label>
            </div>

            <div data-stagger-panel class="rounded border border-slate-700 bg-slate-800/70 p-5 lg:col-span-2">
              <label for="kv-cache-range" class="mb-4 block text-sm font-bold uppercase tracking-widest text-slate-300">${t("ai.field.kv-cache")}</label>
              <input id="kv-cache-range" type="range" min="8" max="128" value="32" class="h-1 w-full cursor-pointer appearance-none rounded-lg bg-slate-700 accent-cyan-500">
              <div class="mt-2 flex justify-between font-mono text-[10px] text-slate-500">
                <span>8GB</span>
                <span id="cache-val" class="text-sm text-cyan-400">32GB</span>
                <span>128GB</span>
              </div>
            </div>
          </div>

          <div data-stagger-panel class="border border-cyan-500/20 bg-cyan-900/10 p-4">
            <label class="block text-[10px] font-bold uppercase text-cyan-500">${t("ai.field.tde-key")}
              <input id="tde-key" type="password" placeholder="${t("ai.placeholder.tde")}" class="mt-2 w-full border-0 border-b border-cyan-500/30 bg-transparent pb-1 text-cyan-100 outline-none placeholder:text-slate-600 focus:border-cyan-400">
            </label>
          </div>

          <button id="sync-ai" class="w-full border border-cyan-500 bg-transparent py-4 font-bold text-cyan-500 transition-colors hover:bg-cyan-500 hover:text-slate-950">
            ${t("ai.button")}
          </button>
        </form>

        <div id="feedback-zone" data-stagger-panel class="rounded-xl border border-slate-800 bg-slate-900 px-4 py-3 font-mono text-xs text-slate-400">${t("ai.feedback.empty")}</div>
      </section>
    `);
  }

  private renderVramMetrics(): string {
    const vram = this.metrics?.vramUtilizationPct ?? 0;
    const offload = this.metrics?.ramOffloadActive ?? false;
    const barColor = vram >= 90 ? "bg-red-500" : vram >= 80 ? "bg-amber-400" : "bg-cyan-400";
    const textColor = vram >= 90 ? "text-red-400" : vram >= 80 ? "text-amber-400" : "text-cyan-400";

    return `
      <div data-stagger-panel class="rounded border border-slate-700 bg-slate-800/40 p-4 space-y-3">
        <div class="flex items-center justify-between gap-3">
          <span class="text-[10px] font-bold uppercase tracking-widest text-slate-400">${t("ai.vram.title")}</span>
          <div class="flex items-center gap-2">
            ${offload ? `<span class="rounded bg-amber-500/20 px-2 py-0.5 text-[10px] font-bold uppercase text-amber-300 animate-pulse">${t("ai.vram.offload.active")}</span>` : ""}
            <button id="btn-refresh-vram" type="button" class="rounded border border-slate-700 bg-slate-800 px-2 py-1 text-[11px] text-slate-300 hover:bg-slate-700">${t("ai.vram.refresh")}</button>
          </div>
        </div>
        <div>
          <div class="mb-1 flex justify-between text-[11px]">
            <span class="text-slate-400">${t("ai.vram.utilization")}</span>
            <span class="font-mono ${textColor}">${vram}%</span>
          </div>
          <div class="h-2 w-full overflow-hidden rounded-full bg-slate-700">
            <div class="h-full rounded-full transition-all ${barColor}" style="width:${vram}%"></div>
          </div>
        </div>
        ${offload ? `
        <p class="text-[11px] text-amber-400/80">${t("ai.vram.offload.desc")}</p>
        ` : ""}
      </div>
    `;
  }

  private bindEvents(): void {
    const range = this.root.getElementById("kv-cache-range") as HTMLInputElement | null;
    const cacheValue = this.root.getElementById("cache-val");
    range?.addEventListener("input", () => {
      if (cacheValue) {
        cacheValue.textContent = `${range.value}GB`;
      }
    });

    this.root.querySelector("form")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.applyConfiguration();
    });

    this.root.getElementById("btn-refresh-vram")?.addEventListener("click", () => {
      void this.refreshAiState();
    });

    this.root.getElementById("btn-refresh-models")?.addEventListener("click", () => {
      void this.refreshAiState();
    });

    this.root.querySelectorAll<HTMLSelectElement>("[data-model-load-mode]").forEach((select) => {
      select.addEventListener("change", () => {
        const alias = select.dataset.modelAlias;
        if (alias) {
          this.modelLoadModes = {
            ...this.modelLoadModes,
            [alias]: this.normalizeLoadMode(select.value),
          };
        }
      });
    });
  }

  private async applyConfiguration(): Promise<void> {
    try {
      const response = await applyAndSeal("config-ai", {
          lora_mode: this.value("lora-mode", "dynamic"),
          kv_cache_size: this.numberValue("kv-cache-range", 32),
          tde_key: this.value("tde-key", ""),
          model_loading_modes: this.modelLoadModes,
      });
      this.showFeedback(response.success ? "success" : "error", response.message);
    } catch (error) {
      this.showFeedback("error", error instanceof Error ? error.message : String(error));
    }
  }

  private renderModelLoadModes(): string {
    const rows = this.models
      .map((model) => {
        const mode = this.modelLoadModes[model.alias] ?? "integrity";
        return `
          <tr class="border-t border-slate-800">
            <td class="py-2 pr-3">
              <div class="font-mono text-xs text-slate-200">${this.escape(model.alias)}</div>
              <div class="text-[10px] uppercase tracking-widest text-slate-500">${this.escape(model.engine)}</div>
            </td>
            <td class="py-2">
              <select data-model-load-mode data-model-alias="${this.escape(model.alias)}" class="w-full rounded border border-slate-700 bg-slate-950 px-2 py-1 text-xs text-cyan-300 outline-none focus:border-cyan-400">
                <option value="integrity" ${mode === "integrity" ? "selected" : ""}>${t("ai.model.mode.integrity")}</option>
                <option value="layer" ${mode === "layer" ? "selected" : ""}>${t("ai.model.mode.layer")}</option>
                <option value="layer-batch" ${mode === "layer-batch" ? "selected" : ""}>${t("ai.model.mode.layer-batch")}</option>
              </select>
            </td>
          </tr>
        `;
      })
      .join("");

    return `
      <section data-stagger-panel class="rounded border border-slate-700 bg-slate-800/70 p-5">
        <div class="mb-3 flex items-center justify-between gap-3">
          <div>
            <h3 class="text-sm font-bold uppercase tracking-widest text-slate-200">${t("ai.model.title")}</h3>
            <p class="text-[11px] text-slate-500">${t("ai.model.subtitle")}</p>
          </div>
          <button id="btn-refresh-models" type="button" class="rounded border border-slate-700 bg-slate-900 px-2 py-1 text-[11px] text-slate-300 hover:bg-slate-700">${t("ai.model.refresh")}</button>
        </div>
        ${
          rows
            ? `<table class="w-full table-fixed text-left"><thead><tr class="text-[10px] uppercase tracking-widest text-slate-500"><th class="w-1/2 pb-2 pr-3">${t("ai.model.column.model")}</th><th class="pb-2">${t("ai.model.column.mode")}</th></tr></thead><tbody>${rows}</tbody></table>`
            : `<div class="rounded border border-slate-700 bg-slate-900/70 px-3 py-2 text-[11px] text-slate-500">${t("ai.model.empty")}</div>`
        }
      </section>
    `;
  }

  private reconcileModelLoadModes(): void {
    const aliases = new Set(this.models.map((model) => model.alias));
    const next: Record<string, ModelLoadMode> = {};
    for (const alias of aliases) {
      next[alias] = this.modelLoadModes[alias] ?? "integrity";
    }
    this.modelLoadModes = next;
  }

  private normalizeLoadMode(value: string): ModelLoadMode {
    return value === "layer" || value === "layer-batch" ? value : "integrity";
  }

  private escape(value: string): string {
    return value
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  private animateGlitchEntrance(): void {
    this.animateEntrance();
    const heading = this.root.querySelector("h2");
    if (!heading) {
      return;
    }
    void gsap.fromTo(
      heading,
      { x: -6, opacity: 0.65, textShadow: "4px 0 rgba(34,211,238,0.65), -4px 0 rgba(244,63,94,0.45)" },
      { x: 0, opacity: 1, textShadow: "0 0 rgba(34,211,238,0)", duration: 0.42, ease: "steps(5)" },
    );
  }

  private value(id: string, fallback: string): string {
    const element = this.root.getElementById(id) as HTMLInputElement | HTMLSelectElement | null;
    const value = element?.value.trim();
    return value ? value : fallback;
  }

  private numberValue(id: string, fallback: number): number {
    const input = this.root.getElementById(id) as HTMLInputElement | null;
    if (!input) {
      return fallback;
    }
    const value = input.valueAsNumber;
    return Number.isFinite(value) ? value : fallback;
  }
}

customElements.define("tachyon-ai-panel", TachyonAIPanel);

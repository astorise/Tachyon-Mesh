import gsap from "gsap";

import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { resilientInvoke as invoke } from "../../utils/network";
import { t } from "../../utils/i18n";
import "./TachyonModelUploadPanel";
import {
  type GpuDistribution,
  type HardwareStrategy,
  type ManifestModelHardwareBinding,
  listManifestModelHardwareBindings,
  normalizeHardwareStrategy,
  writeAiKvCaches,
  writeModelHardwareStrategy,
} from "../../controllers/manifestConfigController";

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

type ClusterHardwareSummary = {
  source: string;
  gpuCount: number;
};

export class TachyonAIPanel extends TachyonConfigDashboard {
  private metrics: RuntimeMetrics | null = null;
  private models: AvailableAiModel[] = [];
  private manifestModelBindings: ManifestModelHardwareBinding[] = [];
  private hardwareSummary: ClusterHardwareSummary | null = null;
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
    const [metrics, hardwareSummary] = await Promise.all([
      invoke<RuntimeMetrics>("get_metrics").catch(() => null),
      invoke<ClusterHardwareSummary>("get_cluster_hardware_summary").catch(() => null),
    ]);
    // Unlike metrics (best-effort), a failed model-list fetch is surfaced: silently
    // swallowing it (`catch(() => [])`) is exactly what made an uploaded-but-
    // unlistable model look like "nothing happened". Keep the previous list on error.
    let modelsError: string | null = null;
    let bindingsError: string | null = null;
    try {
      this.models = await invoke<AvailableAiModel[]>("list_available_models");
    } catch (error) {
      modelsError = error instanceof Error ? error.message : String(error);
    }
    try {
      this.manifestModelBindings = await listManifestModelHardwareBindings();
    } catch (error) {
      bindingsError = error instanceof Error ? error.message : String(error);
    }
    this.metrics = metrics;
    this.hardwareSummary = hardwareSummary;
    this.reconcileModelLoadModes();
    this.render();
    this.bindEvents();
    if (modelsError) {
      this.showFeedback("error", t("ai.model.loadError").replace("{message}", modelsError));
    }
    if (bindingsError) {
      this.showFeedback("error", `Failed to load manifest model bindings: ${bindingsError}`);
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

        ${this.renderHardwareStrategies()}

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

    this.root.querySelectorAll<HTMLButtonElement>("[data-hardware-save]").forEach((button) => {
      button.addEventListener("click", () => {
        const key = button.dataset.hardwareSave;
        if (key) {
          void this.saveHardwareStrategy(key);
        }
      });
    });
  }

  private async applyConfiguration(): Promise<void> {
    try {
      const response = await writeAiKvCaches(this.models.map((model) => model.alias));
      const message = response.success
        ? `${response.message} ${t("ai.feedback.manifestOnly")}`
        : response.message;
      this.showFeedback(response.success ? "success" : "error", message);
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

  private renderHardwareStrategies(): string {
    const bindings = this.manifestModelBindings;
    const gpuHint =
      this.hardwareSummary && this.hardwareSummary.gpuCount > 0
        ? `Discovered GPUs: ${Array.from({ length: this.hardwareSummary.gpuCount }, (_, idx) => idx).join(", ")}`
        : "No GPU reported by cluster hardware summary.";

    return `
      <section data-stagger-panel class="rounded border border-slate-700 bg-slate-800/70 p-5">
        <div class="mb-4 flex flex-col gap-2 md:flex-row md:items-end md:justify-between">
          <div>
            <h3 class="text-sm font-bold uppercase tracking-widest text-slate-200">Model Hardware Strategy</h3>
            <p class="text-[11px] text-slate-500">Manifest-backed routes[].models[].hardware_strategy bindings.</p>
          </div>
          <span class="font-mono text-[11px] text-cyan-300">${this.escape(gpuHint)}</span>
        </div>
        ${
          bindings.length === 0
            ? `<div class="rounded border border-slate-700 bg-slate-900/70 px-3 py-2 text-[11px] text-slate-500">No route model bindings are present in the active manifest.</div>`
            : `<div class="space-y-4">${bindings.map((binding) => this.renderHardwareStrategyCard(binding)).join("")}</div>`
        }
      </section>
    `;
  }

  private renderHardwareStrategyCard(binding: ManifestModelHardwareBinding): string {
    const strategy = binding.hardwareStrategy;
    const key = this.hardwareBindingKey(binding);
    const routeLabel = `${binding.routePath}${binding.routeVersion ? ` @ ${binding.routeVersion}` : ""}`;
    return `
      <article class="rounded border border-slate-700 bg-slate-900/70 p-4">
        <div class="mb-4 flex flex-col gap-1 md:flex-row md:items-start md:justify-between">
          <div>
            <div class="font-mono text-sm text-slate-100">${this.escape(binding.alias)}</div>
            <div class="text-[11px] text-slate-500">${this.escape(routeLabel)} · ${this.escape(binding.device)}</div>
          </div>
          <button type="button" data-hardware-save="${this.escape(key)}" class="rounded border border-cyan-500/60 bg-cyan-500/10 px-3 py-2 text-xs font-semibold text-cyan-200 hover:bg-cyan-500/20">Save strategy</button>
        </div>
        <div class="grid grid-cols-1 gap-3 lg:grid-cols-4">
          <label class="text-[11px] uppercase tracking-widest text-slate-400">Distribution
            <select data-hw-key="${this.escape(key)}" data-hw-field="distribution_mode" class="mt-1 w-full rounded border border-slate-700 bg-slate-950 px-2 py-2 text-xs text-cyan-300">
              ${this.renderDistributionOption("single", strategy.distribution_mode)}
              ${this.renderDistributionOption("tensor_parallelism", strategy.distribution_mode)}
              ${this.renderDistributionOption("pipeline_parallelism", strategy.distribution_mode)}
              ${this.renderDistributionOption("expert_parallelism", strategy.distribution_mode)}
            </select>
          </label>
          <label class="text-[11px] uppercase tracking-widest text-slate-400">Device IDs
            <input data-hw-key="${this.escape(key)}" data-hw-field="device_ids" value="${this.escape(strategy.device_ids.join(","))}" placeholder="0,1" class="mt-1 w-full rounded border border-slate-700 bg-slate-950 px-2 py-2 font-mono text-xs text-slate-200">
          </label>
          <label class="text-[11px] uppercase tracking-widest text-slate-400">Pipeline depth
            <input data-hw-key="${this.escape(key)}" data-hw-field="pipeline_depth" type="number" min="0" value="${strategy.pipeline_depth}" class="mt-1 w-full rounded border border-slate-700 bg-slate-950 px-2 py-2 font-mono text-xs text-slate-200">
          </label>
          <label class="text-[11px] uppercase tracking-widest text-slate-400">Prefill chunk tokens
            <input data-hw-key="${this.escape(key)}" data-hw-field="prefill_chunk_tokens" type="number" min="0" value="${strategy.prefill_chunk_tokens ?? ""}" placeholder="8192" class="mt-1 w-full rounded border border-slate-700 bg-slate-950 px-2 py-2 font-mono text-xs text-slate-200">
          </label>
        </div>
        <div class="mt-3 grid grid-cols-1 gap-3 lg:grid-cols-2">
          <label class="text-[11px] uppercase tracking-widest text-slate-400">Stage layer ranges
            <input data-hw-key="${this.escape(key)}" data-hw-field="stage_layer_ranges" value="${this.escape(this.formatPairs(strategy.stage_layer_ranges))}" placeholder="0-15,16-31" class="mt-1 w-full rounded border border-slate-700 bg-slate-950 px-2 py-2 font-mono text-xs text-slate-200">
          </label>
          <label class="text-[11px] uppercase tracking-widest text-slate-400">Expert device map
            <input data-hw-key="${this.escape(key)}" data-hw-field="expert_device_map" value="${this.escape(this.formatPairs(strategy.expert_device_map))}" placeholder="0:0,1:1" class="mt-1 w-full rounded border border-slate-700 bg-slate-950 px-2 py-2 font-mono text-xs text-slate-200">
          </label>
        </div>
        <div class="mt-3 grid grid-cols-1 gap-3 lg:grid-cols-3">
          ${this.renderHardwareToggle(key, "paged_attention", "Paged attention", strategy.paged_attention)}
          ${this.renderHardwareToggle(key, "cuda_graph_decode", "CUDA graph decode", strategy.cuda_graph_decode)}
          ${this.renderHardwareToggle(key, "flashinfer_attention", "FlashInfer attention", strategy.flashinfer_attention)}
        </div>
        <div class="mt-3 grid grid-cols-1 gap-3 lg:grid-cols-[1fr_12rem]">
          <label class="text-[11px] uppercase tracking-widest text-slate-400">Speculative draft model path
            <input data-hw-key="${this.escape(key)}" data-hw-field="speculative_draft_model_path" value="${this.escape(strategy.speculative_draft_model_path)}" placeholder="/models/draft" class="mt-1 w-full rounded border border-slate-700 bg-slate-950 px-2 py-2 font-mono text-xs text-slate-200">
          </label>
          <label class="text-[11px] uppercase tracking-widest text-slate-400">Draft tokens
            <input data-hw-key="${this.escape(key)}" data-hw-field="speculative_draft_tokens" type="number" min="0" value="${strategy.speculative_draft_tokens}" class="mt-1 w-full rounded border border-slate-700 bg-slate-950 px-2 py-2 font-mono text-xs text-slate-200">
          </label>
        </div>
        <p class="mt-3 text-[11px] text-amber-300/80">Runtime rejections from unsupported Candle paths are returned by manifest apply and shown in the feedback area.</p>
      </article>
    `;
  }

  private renderDistributionOption(value: GpuDistribution, selected: GpuDistribution): string {
    return `<option value="${value}" ${value === selected ? "selected" : ""}>${value}</option>`;
  }

  private renderHardwareToggle(key: string, field: keyof HardwareStrategy, label: string, checked: boolean): string {
    return `
      <label class="flex items-center justify-between gap-3 rounded border border-slate-700 bg-slate-950 px-3 py-2 text-[11px] uppercase tracking-widest text-slate-400">
        <span>${this.escape(label)}</span>
        <input data-hw-key="${this.escape(key)}" data-hw-field="${String(field)}" type="checkbox" ${checked ? "checked" : ""} class="h-4 w-4 accent-cyan-500">
      </label>
    `;
  }

  private async saveHardwareStrategy(key: string): Promise<void> {
    const binding = this.manifestModelBindings.find((item) => this.hardwareBindingKey(item) === key);
    if (!binding) {
      this.showFeedback("error", "Model binding is no longer available in the manifest.");
      return;
    }
    try {
      const strategy = this.readHardwareStrategyFromForm(key);
      const response = await writeModelHardwareStrategy(binding.routePath, binding.alias, strategy);
      this.showFeedback(response.success ? "success" : "error", response.message);
      this.manifestModelBindings = await listManifestModelHardwareBindings();
      this.render();
      this.bindEvents();
    } catch (error) {
      this.showFeedback("error", error instanceof Error ? error.message : String(error));
    }
  }

  private readHardwareStrategyFromForm(key: string): HardwareStrategy {
    const controls = Array.from(this.root.querySelectorAll<HTMLInputElement | HTMLSelectElement>("[data-hw-key]"))
      .filter((control) => control.dataset.hwKey === key);
    const valueFor = (field: string): string => controls.find((control) => control.dataset.hwField === field)?.value ?? "";
    const checkedFor = (field: string): boolean => {
      const control = controls.find((item) => item.dataset.hwField === field) as HTMLInputElement | undefined;
      return control?.checked === true;
    };

    return normalizeHardwareStrategy({
      distribution_mode: this.normalizeDistribution(valueFor("distribution_mode")),
      device_ids: this.parseNumberList(valueFor("device_ids")),
      stage_layer_ranges: this.parsePairs(valueFor("stage_layer_ranges")),
      expert_device_map: this.parsePairs(valueFor("expert_device_map")),
      pipeline_depth: this.parseNonNegativeInteger(valueFor("pipeline_depth")),
      paged_attention: checkedFor("paged_attention"),
      cuda_graph_decode: checkedFor("cuda_graph_decode"),
      flashinfer_attention: checkedFor("flashinfer_attention"),
      prefill_chunk_tokens: valueFor("prefill_chunk_tokens") === "" ? undefined : this.parseNonNegativeInteger(valueFor("prefill_chunk_tokens")),
      speculative_draft_model_path: valueFor("speculative_draft_model_path").trim(),
      speculative_draft_tokens: this.parseNonNegativeInteger(valueFor("speculative_draft_tokens")),
    });
  }

  private hardwareBindingKey(binding: ManifestModelHardwareBinding): string {
    return `${binding.routePath}::${binding.alias}`;
  }

  private normalizeDistribution(value: string): GpuDistribution {
    return value === "tensor_parallelism" || value === "pipeline_parallelism" || value === "expert_parallelism" ? value : "single";
  }

  private parseNumberList(value: string): number[] {
    return value.split(",").map((item) => this.parseNonNegativeInteger(item)).filter((item, index, list) => item > 0 || list.indexOf(item) === index);
  }

  private parsePairs(value: string): Array<[number, number]> {
    return value
      .split(",")
      .map((item) => item.trim())
      .filter(Boolean)
      .map((item) => {
        const [left, right] = item.split(/[:-]/);
        return [this.parseNonNegativeInteger(left), this.parseNonNegativeInteger(right)] as [number, number];
      });
  }

  private formatPairs(pairs: Array<[number, number]>): string {
    return pairs.map(([left, right]) => `${left}:${right}`).join(",");
  }

  private parseNonNegativeInteger(value: string): number {
    const numeric = Number.parseInt(value, 10);
    return Number.isFinite(numeric) && numeric > 0 ? numeric : 0;
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

}

customElements.define("tachyon-ai-panel", TachyonAIPanel);

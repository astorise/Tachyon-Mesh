import gsap from "gsap";

import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { resilientInvoke as invoke } from "../../utils/network";

type ApplyConfigurationResponse = {
  success: boolean;
  message: string;
};

export class TachyonAIPanel extends TachyonConfigDashboard {
  connectedCallback(): void {
    this.render();
    this.bindEvents();
    this.animateGlitchEntrance();
  }

  private render(): void {
    this.renderTemplate(`
      <section class="p-6 space-y-8 text-slate-300">
        <header data-stagger-panel class="flex flex-col gap-4 md:flex-row md:items-end md:justify-between">
          <div>
            <h2 class="text-3xl font-light text-cyan-400">AI Mesh <span class="font-bold text-slate-100">Compute</span></h2>
            <p class="text-xs font-mono text-slate-500">WIT: config-ai / Multi-GPU multiplexing</p>
          </div>
          <div class="text-left md:text-right">
            <span class="block text-[10px] uppercase text-cyan-500/70">Accelerator Status</span>
            <span class="font-mono text-xs text-emerald-400">ENCLAVE SECURE (TEE)</span>
          </div>
        </header>

        <form class="space-y-6">
          <div class="grid grid-cols-1 gap-6 lg:grid-cols-3">
            <div data-stagger-panel class="space-y-4 rounded border border-slate-700 bg-slate-800/70 p-5">
              <label class="block text-sm font-bold uppercase tracking-widest text-slate-300">LoRA Multiplexing
                <select id="lora-mode" class="mt-3 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm text-cyan-300 outline-none transition-colors focus:border-cyan-400">
                  <option value="dynamic">Dynamic Allocation</option>
                  <option value="static">Static High Priority</option>
                </select>
              </label>
            </div>

            <div data-stagger-panel class="rounded border border-slate-700 bg-slate-800/70 p-5 lg:col-span-2">
              <label for="kv-cache-range" class="mb-4 block text-sm font-bold uppercase tracking-widest text-slate-300">Edge KV Cache</label>
              <input id="kv-cache-range" type="range" min="8" max="128" value="32" class="h-1 w-full cursor-pointer appearance-none rounded-lg bg-slate-700 accent-cyan-500">
              <div class="mt-2 flex justify-between font-mono text-[10px] text-slate-500">
                <span>8GB</span>
                <span id="cache-val" class="text-sm text-cyan-400">32GB</span>
                <span>128GB</span>
              </div>
            </div>
          </div>

          <div data-stagger-panel class="border border-cyan-500/20 bg-cyan-900/10 p-4">
            <label class="block text-[10px] font-bold uppercase text-cyan-500">TDE Master Key
              <input id="tde-key" type="password" placeholder="Encrypted storage key" class="mt-2 w-full border-0 border-b border-cyan-500/30 bg-transparent pb-1 text-cyan-100 outline-none placeholder:text-slate-600 focus:border-cyan-400">
            </label>
          </div>

          <button id="sync-ai" class="w-full border border-cyan-500 bg-transparent py-4 font-bold text-cyan-500 transition-colors hover:bg-cyan-500 hover:text-slate-950">
            Synchronize AI Control Plane
          </button>
        </form>

        <div id="feedback-zone" data-stagger-panel class="rounded-xl border border-slate-800 bg-slate-900 px-4 py-3 font-mono text-xs text-slate-400">Awaiting AI control plane sync.</div>
      </section>
    `);
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
  }

  private async applyConfiguration(): Promise<void> {
    try {
      const response = await invoke<ApplyConfigurationResponse>("apply_configuration", {
        domain: "config-ai",
        payload: {
          lora_mode: this.value("lora-mode", "dynamic"),
          kv_cache_size: this.numberValue("kv-cache-range", 32),
          tde_key: this.value("tde-key", ""),
        },
      });
      this.showFeedback(response.success ? "success" : "error", response.message);
    } catch (error) {
      this.showFeedback("error", error instanceof Error ? error.message : String(error));
    }
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

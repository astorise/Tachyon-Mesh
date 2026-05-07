import gsap from "gsap";

import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";

type OverviewMetric = {
  label: string;
  value: number;
  suffix: string;
  detail: string;
};

const metrics: OverviewMetric[] = [
  { label: "Active Edge Nodes", value: 12, suffix: "", detail: "Mesh members reporting healthy control-plane heartbeats" },
  { label: "Global Wasm Instances", value: 48, suffix: "", detail: "Component workloads currently admitted across the fleet" },
  { label: "AI/GPU Utilization", value: 73, suffix: "%", detail: "Accelerator allocation across active AI routing targets" },
];

export class TachyonOverviewPanel extends TachyonConfigDashboard {
  connectedCallback(): void {
    this.render();
    this.animateEntrance();
    this.animateCounters();
  }

  private render(): void {
    this.renderTemplate(`
      <section class="p-6 space-y-8 text-slate-300">
        <header data-stagger-panel class="flex flex-col gap-3 md:flex-row md:items-end md:justify-between">
          <div>
            <h2 class="text-3xl font-light text-cyan-400">Global <span class="font-bold text-slate-100">Overview</span></h2>
            <p class="text-xs font-mono text-slate-500">Mesh telemetry / boot sequence snapshot</p>
          </div>
          <div class="rounded border border-cyan-500/30 bg-cyan-500/10 px-3 py-2 font-mono text-xs text-cyan-300">
            CONTROL PLANE ONLINE
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
                    <span data-counter="${metric.value}" data-suffix="${metric.suffix}">0${metric.suffix}</span>
                  </div>
                  <p class="mt-4 min-h-12 text-sm leading-6 text-slate-500">${metric.detail}</p>
                </article>
              `,
            )
            .join("")}
        </div>

        <div data-stagger-panel class="grid grid-cols-1 gap-4 rounded-lg border border-slate-800 bg-slate-900/70 p-5 font-mono text-xs text-slate-400 md:grid-cols-3">
          <div><span class="text-cyan-400">routing</span> sealed</div>
          <div><span class="text-cyan-400">identity</span> enforced</div>
          <div><span class="text-cyan-400">observability</span> ready</div>
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
}

customElements.define("tachyon-overview-panel", TachyonOverviewPanel);

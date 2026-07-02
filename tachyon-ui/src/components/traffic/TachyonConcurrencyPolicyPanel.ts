import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { resilientInvoke as invoke } from "../../utils/network";
import {
  listManifestRoutes,
  writeRouteConcurrencyPolicy,
  type ManifestRouteOption,
} from "../../controllers/manifestConfigController";
import "../base/TachyonRiskBadge";

type Recommendation = {
  concurrency: { mode: string; onConflict: string; lockTtlMs?: number };
  consistency: { readMode: string; writeMode: string };
  coordination: { mode: string; writeIsolation: string };
  rationale: string;
  riskLevel: "low" | "medium" | "high";
  tradeOffs: string[];
};

const PATTERNS = [
  { value: "interactive", label: "Interactive (low-latency request/response)" },
  { value: "batch", label: "Batch (scheduled jobs)" },
  { value: "stateful", label: "Stateful (shared mutable state)" },
  { value: "etl", label: "ETL (data pipelines)" },
  { value: "scheduler", label: "Scheduler (singleton coordinator)" },
];

const CONCURRENCY_MODES = [
  { value: "unrestricted", label: "Unrestricted (N parallel)" },
  { value: "node-singleton", label: "Node singleton (1 per node)" },
  { value: "mesh-singleton", label: "Mesh singleton (1 mesh-wide)" },
  { value: "mesh-leader", label: "Mesh leader (elected node, eventual)" },
  {
    value: "mesh-leader-strict",
    label: "Mesh leader strict (elected + lock, strong)",
  },
];

const CONFLICT_POLICIES = [
  { value: "queue", label: "Queue (wait for slot)" },
  { value: "reject", label: "Reject (HTTP 409 / 503)" },
  { value: "drop", label: "Drop (silent discard)" },
];

const WRITE_MODES = [
  { value: "last_write_wins", label: "Last write wins (simple, racy)" },
  { value: "optimistic_etag", label: "Optimistic ETag (fail on conflict)" },
  { value: "pessimistic_lock", label: "Pessimistic lock (serialize writes)" },
  { value: "none", label: "None (read-only volume)" },
];

const READ_MODES = [
  { value: "snapshot", label: "Snapshot (per-invocation copy)" },
  { value: "live", label: "Live (shared, read-only)" },
];

type Selection = {
  concurrency: string;
  onConflict: string;
  readMode: string;
  writeMode: string;
};

function deriveRisk(sel: Selection): { level: "low" | "medium" | "high"; tooltip: string; sim: string } {
  if (sel.concurrency === "unrestricted" && sel.writeMode === "last_write_wins") {
    return {
      level: "high",
      tooltip:
        "Concurrent invocations will silently overwrite each other's writes — last writer wins and earlier results vanish without warning.",
      sim: "unrestricted-lww",
    };
  }
  if (sel.concurrency === "unrestricted" && sel.writeMode === "pessimistic_lock") {
    return {
      level: "high",
      tooltip:
        "Pessimistic locking requires a singleton concurrency mode. As declared, the lock provides no protection.",
      sim: "lock-without-singleton",
    };
  }
  if (sel.writeMode === "optimistic_etag") {
    return {
      level: "medium",
      tooltip:
        "Concurrent writers may fail with HTTP 412 Precondition Failed on conflict — clients must retry.",
      sim: "etag-conflict",
    };
  }
  if (
    sel.concurrency === "mesh-singleton" ||
    sel.concurrency === "mesh-leader" ||
    sel.concurrency === "mesh-leader-strict"
  ) {
    const tooltip =
      sel.concurrency === "mesh-leader-strict"
        ? "Elected leader + distributed lock — strongest guarantee, one extra redb round-trip per invocation."
        : "All invocations serialize through a single node — strong consistency with added latency under load.";
    return {
      level: "low",
      tooltip,
      sim:
        sel.concurrency === "mesh-leader-strict"
          ? "mesh-leader-strict"
          : "mesh-singleton-queue",
    };
  }
  return {
    level: "low",
    tooltip: "Default permissive policy — safe for stateless or per-tenant routes.",
    sim: "default",
  };
}

export class TachyonConcurrencyPolicyPanel extends TachyonConfigDashboard {
  private routePath = "";
  private routes: ManifestRouteOption[] = [];
  private selection: Selection = {
    concurrency: "unrestricted",
    onConflict: "queue",
    readMode: "snapshot",
    writeMode: "last_write_wins",
  };
  private recommendation: Recommendation | null = null;
  private feedback = "";

  static get observedAttributes() {
    return ["route-path"];
  }

  attributeChangedCallback(name: string, _old: string, value: string): void {
    if (name === "route-path") this.routePath = value;
  }

  connectedCallback(): void {
    this.routePath = this.getAttribute("route-path") ?? "";
    this.render();
    this.bindEvents();
    void this.loadRoutes();
  }

  private async loadRoutes(): Promise<void> {
    try {
      this.routes = await listManifestRoutes();
      if (!this.routePath) {
        this.routePath = this.routes[0]?.path ?? "";
      }
    } catch (error) {
      this.feedback = error instanceof Error ? error.message : String(error);
      this.routes = [];
    }
    this.render();
    this.bindEvents();
  }

  private render(): void {
    const risk = deriveRisk(this.selection);
    const incompatible =
      this.selection.concurrency === "unrestricted" &&
      this.selection.writeMode === "pessimistic_lock";

    this.renderTemplate(`
      <section class="mt-3 rounded-lg border border-slate-700 bg-slate-900/80 p-4 space-y-3">
        <header class="flex items-center justify-between">
          <h4 class="text-xs font-bold uppercase tracking-widest text-slate-400">Concurrency Policy</h4>
          <tachyon-risk-badge
            level="${risk.level}"
            tooltip="${this.escAttr(risk.tooltip)}"
            sim-scenario="${risk.sim}"
          ></tachyon-risk-badge>
        </header>

        ${incompatible ? `
          <div class="rounded border border-amber-700/60 bg-amber-900/30 p-2 text-xs text-amber-200">
            ⚠ Pessimistic locking requires a singleton concurrency mode.
            <button id="btn-fix-incompatible"
              class="ml-2 underline text-amber-100 hover:text-amber-50">
              Switch concurrency to mesh-singleton
            </button>
          </div>` : ""}

        <div class="grid grid-cols-2 gap-3 text-xs">
          <label class="flex flex-col gap-1 md:col-span-2">
            <span class="text-slate-500 uppercase tracking-widest">Manifest route</span>
            <select id="sel-route-path" class="rounded border border-slate-600 bg-slate-900 px-2 py-1 text-slate-200">
              ${this.renderRouteOptions()}
            </select>
          </label>
          <label class="flex flex-col gap-1">
            <span class="text-slate-500 uppercase tracking-widest">Concurrency mode</span>
            <select id="sel-concurrency" class="rounded border border-slate-600 bg-slate-900 px-2 py-1 text-slate-200">
              ${CONCURRENCY_MODES.map((m) => `<option value="${m.value}" data-sim-scenario="mode-${m.value}" ${this.selection.concurrency === m.value ? "selected" : ""}>${m.label}</option>`).join("")}
            </select>
          </label>
          <label class="flex flex-col gap-1">
            <span class="text-slate-500 uppercase tracking-widest">On conflict</span>
            <select id="sel-on-conflict" class="rounded border border-slate-600 bg-slate-900 px-2 py-1 text-slate-200">
              ${CONFLICT_POLICIES.map((p) => `<option value="${p.value}" data-sim-scenario="conflict-${p.value}" ${this.selection.onConflict === p.value ? "selected" : ""}>${p.label}</option>`).join("")}
            </select>
          </label>
          <label class="flex flex-col gap-1">
            <span class="text-slate-500 uppercase tracking-widest">Volume read mode</span>
            <select id="sel-read-mode" class="rounded border border-slate-600 bg-slate-900 px-2 py-1 text-slate-200">
              ${READ_MODES.map((m) => `<option value="${m.value}" data-sim-scenario="read-${m.value}" ${this.selection.readMode === m.value ? "selected" : ""}>${m.label}</option>`).join("")}
            </select>
          </label>
          <label class="flex flex-col gap-1">
            <span class="text-slate-500 uppercase tracking-widest">Volume write mode</span>
            <select id="sel-write-mode" class="rounded border border-slate-600 bg-slate-900 px-2 py-1 text-slate-200">
              ${WRITE_MODES.map((m) => `<option value="${m.value}" data-sim-scenario="write-${m.value}" ${this.selection.writeMode === m.value ? "selected" : ""}>${m.label}</option>`).join("")}
            </select>
          </label>
        </div>

        <details class="rounded border border-slate-700/60 bg-slate-800/40 p-2 text-xs">
          <summary class="cursor-pointer text-slate-400">Get a recommendation from a usage pattern</summary>
          <div class="mt-2 flex items-center gap-2">
            <select id="sel-pattern" class="rounded border border-slate-600 bg-slate-900 px-2 py-1 text-slate-200">
              ${PATTERNS.map((p) => `<option value="${p.value}">${p.label}</option>`).join("")}
            </select>
            <label class="flex items-center gap-1 text-slate-400">
              <input type="checkbox" id="chk-shared-state" /> writes shared state
            </label>
            <button id="btn-recommend"
              class="rounded border border-cyan-600/60 bg-cyan-700/30 px-2 py-1 text-cyan-200 hover:bg-cyan-700/40">
              Recommend
            </button>
          </div>
          ${this.recommendation ? this.renderRecommendation(this.recommendation) : ""}
        </details>

        <button id="btn-apply-route" ${this.routePath ? "" : "disabled"}
          class="rounded border border-emerald-700/60 bg-emerald-800/30 px-3 py-2 text-xs font-semibold text-emerald-200 hover:bg-emerald-800/40 disabled:cursor-not-allowed disabled:opacity-50">
          Apply to route
        </button>

        ${this.feedback ? `<p class="text-xs text-slate-400 font-mono">${this.escHtml(this.feedback)}</p>` : ""}
      </section>
    `);
  }

  private renderRouteOptions(): string {
    if (this.routes.length === 0 && this.routePath) {
      return `<option value="${this.escAttr(this.routePath)}">${this.escHtml(this.routePath)}</option>`;
    }
    if (this.routes.length === 0) {
      return `<option value="">No manifest routes</option>`;
    }
    return this.routes
      .map((route) => {
        const label = [route.path, route.name, route.version].filter(Boolean).join(" - ");
        return `<option value="${this.escAttr(route.path)}"${route.path === this.routePath ? " selected" : ""}>${this.escHtml(label)}</option>`;
      })
      .join("");
  }

  private renderRecommendation(r: Recommendation): string {
    const tradeOffs = r.tradeOffs
      .map((t) => `<li class="text-slate-400">• ${this.escHtml(t)}</li>`)
      .join("");
    return `
      <div class="mt-3 space-y-1.5 rounded border border-slate-700/60 bg-slate-900/60 p-2">
        <div class="flex items-center gap-2">
          <tachyon-risk-badge level="${r.riskLevel}" tooltip="${this.escAttr(r.rationale)}" sim-scenario="rec-${r.concurrency.mode}"></tachyon-risk-badge>
          <span class="text-slate-300">${this.escHtml(r.rationale)}</span>
        </div>
        <div class="font-mono text-slate-400">
          concurrency=${r.concurrency.mode} / on_conflict=${r.concurrency.onConflict}<br/>
          consistency.read=${r.consistency.readMode} / write=${r.consistency.writeMode}<br/>
          coordination=${r.coordination.mode} / write_isolation=${r.coordination.writeIsolation}
        </div>
        <ul class="text-xs">${tradeOffs}</ul>
        <button id="btn-apply-recommendation"
          class="rounded border border-emerald-700/60 bg-emerald-800/30 px-2 py-1 text-xs text-emerald-200 hover:bg-emerald-800/40">
          Apply to selection
        </button>
      </div>`;
  }

  private bindEvents(): void {
    this.root.getElementById("sel-route-path")?.addEventListener("change", (e) => {
      this.routePath = (e.target as HTMLSelectElement).value;
      this.render();
      this.bindEvents();
    });
    this.root.getElementById("sel-concurrency")?.addEventListener("change", (e) => {
      this.selection.concurrency = (e.target as HTMLSelectElement).value;
      this.render();
      this.bindEvents();
    });
    this.root.getElementById("sel-on-conflict")?.addEventListener("change", (e) => {
      this.selection.onConflict = (e.target as HTMLSelectElement).value;
      this.render();
      this.bindEvents();
    });
    this.root.getElementById("sel-read-mode")?.addEventListener("change", (e) => {
      this.selection.readMode = (e.target as HTMLSelectElement).value;
      this.render();
      this.bindEvents();
    });
    this.root.getElementById("sel-write-mode")?.addEventListener("change", (e) => {
      this.selection.writeMode = (e.target as HTMLSelectElement).value;
      this.render();
      this.bindEvents();
    });
    this.root.getElementById("btn-fix-incompatible")?.addEventListener("click", () => {
      this.selection.concurrency = "mesh-singleton";
      this.render();
      this.bindEvents();
    });
    this.root.getElementById("btn-recommend")?.addEventListener("click", () => {
      void this.fetchRecommendation();
    });
    this.root.getElementById("btn-apply-recommendation")?.addEventListener("click", () => {
      if (!this.recommendation) return;
      this.selection.concurrency = this.recommendation.concurrency.mode;
      this.selection.onConflict = this.recommendation.concurrency.onConflict;
      this.selection.readMode = this.recommendation.consistency.readMode;
      this.selection.writeMode = this.recommendation.consistency.writeMode;
      this.render();
      this.bindEvents();
    });
    this.root.getElementById("btn-apply-route")?.addEventListener("click", () => {
      void this.applyToRoute();
    });
  }

  private async fetchRecommendation(): Promise<void> {
    const pattern = (this.root.getElementById("sel-pattern") as HTMLSelectElement | null)?.value ?? "interactive";
    const writesSharedState = (this.root.getElementById("chk-shared-state") as HTMLInputElement | null)?.checked ?? false;
    try {
      this.recommendation = await invoke<Recommendation>("recommend_concurrency_policy", {
        pattern,
        writesSharedState,
      });
      this.feedback = "";
    } catch (error) {
      this.feedback = error instanceof Error ? error.message : String(error);
    }
    this.render();
    this.bindEvents();
  }

  private async applyToRoute(): Promise<void> {
    if (!this.routePath) {
      this.feedback = "No manifest route is available for concurrency policy.";
      this.render();
      this.bindEvents();
      return;
    }
    try {
      const response = await writeRouteConcurrencyPolicy(
        this.routePath,
        {
          mode: this.selection.concurrency,
          on_conflict: this.selection.onConflict,
        },
        {
          read_mode: this.selection.readMode as "snapshot" | "live",
          write_mode: this.selection.writeMode as "last_write_wins" | "optimistic_etag" | "pessimistic_lock" | "none",
        },
      );
      this.feedback = response.message;
    } catch (error) {
      this.feedback = error instanceof Error ? error.message : String(error);
    }
    this.render();
    this.bindEvents();
  }

  private escHtml(str: string): string {
    return str.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
  }

  private escAttr(str: string): string {
    return str.replace(/"/g, "&quot;").replace(/</g, "&lt;");
  }
}

customElements.define("tachyon-concurrency-policy-panel", TachyonConcurrencyPolicyPanel);

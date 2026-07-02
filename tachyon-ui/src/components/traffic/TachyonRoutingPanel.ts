import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { el } from "../../utils/dom-safe";
import { resilientInvoke as invoke } from "../../utils/network";
import { t } from "../../utils/i18n";
import {
  getManifestOperatorConfig,
  writeManifestOperatorConfig,
  writeRouteRequiresTee,
  type ManifestOperatorConfig,
} from "../../controllers/manifestConfigController";
import "./TachyonVolumesPanel";
import "./TachyonConcurrencyPolicyPanel";
import "../routing/TachyonScopesPanel";

type MeshRouteSummary = {
  name: string;
  path: string;
  role: string;
  targetCount: number;
  requiresTee: boolean;
  encryptedVolumeCount: number;
  allowAllScopes: boolean;
};

type MeshGraphSnapshot = {
  source: string;
  status: string;
  routes: MeshRouteSummary[];
  batchTargets: string[];
};

export class TachyonRoutingPanel extends TachyonConfigDashboard {
  private snapshot: MeshGraphSnapshot | null = null;
  private expandedRoute: string | null = null;
  private operatorConfig: ManifestOperatorConfig | null = null;

  async connectedCallback(): Promise<void> {
    this.render();
    this.bindForm();
    this.animateEntrance();
    try {
      this.snapshot = await invoke<MeshGraphSnapshot>("get_mesh_graph");
    } catch {
      this.snapshot = null;
    }
    try {
      this.operatorConfig = await getManifestOperatorConfig();
    } catch {
      this.operatorConfig = null;
    }
    this.render();
    this.bindForm();
  }

  private bindForm(): void {
    this.root.querySelector<HTMLFormElement>("#operator-routing-form")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.applyOperatorConfig();
    });
  }

  private render(): void {
    this.renderTemplate(`
      <section class="p-6 space-y-6 text-slate-300">
        <div data-stagger-panel class="border-l-4 border-cyan-500 pl-4">
          <h2 class="text-2xl font-bold text-slate-100">Gateway Routing</h2>
          <p class="text-slate-400 text-sm font-mono">Domain: routing.wit</p>
        </div>

        <article data-stagger-panel class="rounded-lg border border-slate-800 bg-slate-900/70 p-5">
          <h3 class="mb-3 text-sm font-semibold uppercase tracking-widest text-cyan-300">${t("routing.preview.title")}</h3>
          ${this.renderSnapshot()}
        </article>

        ${this.renderOperatorConfig()}

        <div id="feedback-zone" data-stagger-panel class="mt-4 rounded-xl border border-slate-800 bg-slate-900 px-4 py-3 font-mono text-xs text-slate-400">Awaiting manifest routing update.</div>
      </section>
    `);
    this.populateSnapshotRows();
  }

  private renderOperatorConfig(): string {
    const config = this.operatorConfig ?? {
      layer4: { tcp: [], udp: [] },
      teeBackend: null,
      telemetrySampleRate: 1,
      instancePoolMaxMemoryBytes: null,
      cloudSyncEndpoint: "",
      batchTargets: [],
      requireScopes: false,
    };
    const tcp = this.formatLayer4(config.layer4.tcp ?? []);
    const udp = this.formatLayer4(config.layer4.udp ?? []);
    const teeKind = config.teeBackend?.kind ?? "none";
    const keepEndpoint = config.teeBackend?.kind === "enarx" ? config.teeBackend.keep_endpoint : "";
    return `
      <form id="operator-routing-form" data-stagger-panel class="space-y-4 rounded-lg border border-slate-700 bg-slate-800/40 p-5">
        <h3 class="text-sm font-semibold uppercase tracking-widest text-cyan-300">Manifest routing controls</h3>
        <div class="grid gap-4 md:grid-cols-2">
          <label class="block text-xs uppercase tracking-widest text-cyan-500">TCP bindings
            <textarea id="layer4-tcp" rows="3" placeholder="443=/api&#10;1883=/mqtt" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 font-mono text-xs text-slate-200">${this.escape(tcp)}</textarea>
          </label>
          <label class="block text-xs uppercase tracking-widest text-cyan-500">UDP bindings
            <textarea id="layer4-udp" rows="3" placeholder="5060=/voip" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 font-mono text-xs text-slate-200">${this.escape(udp)}</textarea>
          </label>
          <label class="block text-xs uppercase tracking-widest text-cyan-500">TEE backend
            <select id="tee-backend" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm text-slate-200">
              <option value="none"${teeKind === "none" ? " selected" : ""}>None</option>
              <option value="local-enclave"${teeKind === "local-enclave" ? " selected" : ""}>LocalEnclave</option>
              <option value="enarx"${teeKind === "enarx" ? " selected" : ""}>Enarx</option>
            </select>
          </label>
          <label class="block text-xs uppercase tracking-widest text-cyan-500">Enarx keep endpoint
            <input id="tee-keep-endpoint" type="text" value="${this.escape(keepEndpoint)}" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 font-mono text-xs text-slate-200">
          </label>
          <label class="block text-xs uppercase tracking-widest text-cyan-500">Telemetry sample rate
            <input id="telemetry-sample-rate" type="number" min="0" max="1" step="0.01" value="${config.telemetrySampleRate}" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm text-slate-200">
          </label>
          <label class="block text-xs uppercase tracking-widest text-cyan-500">Instance pool memory bytes
            <input id="instance-pool-memory" type="number" min="0" step="1" value="${config.instancePoolMaxMemoryBytes ?? ""}" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm text-slate-200">
          </label>
          <label class="block text-xs uppercase tracking-widest text-cyan-500">Cloud sync endpoint
            <input id="cloud-sync-endpoint" type="url" value="${this.escape(config.cloudSyncEndpoint)}" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 text-sm text-slate-200">
          </label>
          <label class="block text-xs uppercase tracking-widest text-cyan-500">Batch targets
            <input id="batch-targets" type="text" value="${this.escape(config.batchTargets.join(", "))}" placeholder="cleanup, reporting" class="mt-1 w-full rounded border border-slate-600 bg-slate-900 p-2 font-mono text-xs text-slate-200">
          </label>
        </div>
        <label class="flex items-center gap-2 text-xs uppercase tracking-widest text-cyan-500">
          <input id="require-scopes" type="checkbox"${config.requireScopes ? " checked" : ""}>
          Require explicit import scopes
        </label>
        <button type="submit" class="rounded border border-cyan-500 px-4 py-2 text-xs font-bold text-cyan-300 hover:bg-cyan-500 hover:text-slate-950">Save manifest controls</button>
      </form>
    `;
  }

  private renderSnapshot(): string {
    if (!this.snapshot || this.snapshot.routes.length === 0) {
      return `<p class="text-xs text-slate-500">${t("routing.preview.empty")}</p>`;
    }
    // Static structure — rows populated via DOM API in populateSnapshotRows().
    return `
      <table class="w-full text-xs">
        <thead class="text-slate-500 uppercase tracking-widest">
          <tr><th class="text-left pb-2 pr-4">${t("routing.preview.column.name")}</th><th class="text-left pb-2 pr-4">${t("routing.preview.column.path")}</th><th class="text-left pb-2 pr-4">${t("routing.preview.column.targets")}</th><th class="text-left pb-2 pr-4">${t("routing.preview.column.tee")}</th><th class="text-left pb-2 pr-2">Scopes</th><th></th></tr>
        </thead>
        <tbody id="routing-snapshot-rows"></tbody>
      </table>
    `;
  }

  private populateSnapshotRows(): void {
    const tbody = this.root.getElementById("routing-snapshot-rows");
    if (!tbody || !this.snapshot) return;

    const rows: Element[] = [];
    for (const route of this.snapshot.routes) {
      const isExpanded = this.expandedRoute === route.path;
      const scopeBadge = route.allowAllScopes
        ? el("span", {
            class: "rounded-full bg-amber-500/20 border border-amber-500/40 px-1.5 py-0.5 text-[9px] font-semibold text-amber-300 uppercase tracking-widest cursor-help",
            title: "This deployment grants all WIT imports. Click to configure scopes.",
          }, "allow-all")
        : el("span", {
            class: "rounded-full bg-emerald-500/20 border border-emerald-500/40 px-1.5 py-0.5 text-[9px] font-semibold text-emerald-300 uppercase tracking-widest",
          }, "scoped");
      const tr = el("tr", { class: "cursor-pointer hover:bg-slate-800/40", "data-route-path": route.path },
        el("td", { class: "py-1 pr-4 text-cyan-300" }, route.name),
        el("td", { class: "py-1 pr-4 font-mono text-slate-300" }, route.path),
        el("td", { class: "py-1 pr-4 text-slate-300" }, String(route.targetCount)),
        el("td", { class: "py-1 text-slate-300" },
          el("button", {
            class: "rounded border border-slate-700 px-2 py-1 text-[10px] text-slate-300 hover:border-cyan-500 hover:text-cyan-200",
            "data-tee-route": route.path,
            type: "button",
          }, route.requiresTee ? "yes" : "no"),
        ),
        el("td", { class: "py-1 pr-2" }, scopeBadge),
        el("td", { class: "py-1 text-slate-400 text-xs" }, isExpanded ? "▲" : "▼"),
      );
      tr.addEventListener("click", () => {
        this.expandedRoute = isExpanded ? null : route.path;
        this.populateSnapshotRows();
      });
      rows.push(tr);

      if (isExpanded) {
        const wrapper = document.createElement("div");
        wrapper.className = "space-y-2";
        const concPanel = document.createElement("tachyon-concurrency-policy-panel") as HTMLElement;
        concPanel.setAttribute("route-path", route.path);
        const volPanel = document.createElement("tachyon-volumes-panel") as HTMLElement;
        volPanel.setAttribute("route-path", route.path);
        const scopesPanel = document.createElement("tachyon-scopes-panel") as HTMLElement;
        scopesPanel.setAttribute("route-path", route.path);
        wrapper.appendChild(concPanel);
        wrapper.appendChild(volPanel);
        wrapper.appendChild(scopesPanel);
        const detailRow = el("tr", {},
          el("td", { colspan: "6", class: "pb-2" }, wrapper),
        );
        rows.push(detailRow);
      }
    }

    tbody.replaceChildren(...rows);
    tbody.querySelectorAll<HTMLButtonElement>("[data-tee-route]").forEach((button) => {
      button.addEventListener("click", (event) => {
        event.stopPropagation();
        const routePath = button.dataset.teeRoute ?? "";
        const route = this.snapshot?.routes.find((item) => item.path === routePath);
        if (!route) return;
        void this.applyRouteTee(route.path, !route.requiresTee);
      });
    });
  }

  private async applyRouteTee(routePath: string, requiresTee: boolean): Promise<void> {
    try {
      const response = await writeRouteRequiresTee(routePath, requiresTee);
      this.showFeedback(response.success ? "success" : "error", response.message);
      if (this.snapshot) {
        this.snapshot = {
          ...this.snapshot,
          routes: this.snapshot.routes.map((route) => route.path === routePath ? { ...route, requiresTee } : route),
        };
        this.populateSnapshotRows();
      }
    } catch (error) {
      this.showFeedback("error", error instanceof Error ? error.message : String(error));
    }
  }

  private async applyOperatorConfig(): Promise<void> {
    try {
      const teeKind = this.value("tee-backend", "none");
      const teeBackend = teeKind === "local-enclave"
        ? { kind: "local-enclave" as const }
        : teeKind === "enarx"
          ? { kind: "enarx" as const, keep_endpoint: this.value("tee-keep-endpoint", "") }
          : null;
      const response = await writeManifestOperatorConfig({
        layer4: {
          tcp: this.parseLayer4("layer4-tcp"),
          udp: this.parseLayer4("layer4-udp"),
        },
        teeBackend,
        telemetrySampleRate: this.numberValue("telemetry-sample-rate", 1),
        instancePoolMaxMemoryBytes: this.optionalNumberValue("instance-pool-memory"),
        cloudSyncEndpoint: this.value("cloud-sync-endpoint", ""),
        batchTargets: this.value("batch-targets", "").split(","),
        requireScopes: (this.root.getElementById("require-scopes") as HTMLInputElement | null)?.checked ?? false,
      });
      this.operatorConfig = await getManifestOperatorConfig();
      this.render();
      this.bindForm();
      this.showFeedback(response.success ? "success" : "error", response.message);
    } catch (error) {
      this.showFeedback("error", error instanceof Error ? error.message : String(error));
    }
  }

  private value(id: string, fallback: string): string {
    const value = (this.root.getElementById(id) as HTMLInputElement | null)?.value.trim();
    return value ? value : fallback;
  }

  private numberValue(id: string, fallback: number): number {
    const value = Number.parseFloat(this.value(id, String(fallback)));
    return Number.isFinite(value) ? value : fallback;
  }

  private optionalNumberValue(id: string): number | null {
    const value = this.value(id, "");
    if (!value) return null;
    const parsed = Number.parseInt(value, 10);
    return Number.isFinite(parsed) && parsed > 0 ? parsed : null;
  }

  private parseLayer4(id: string): Array<{ port: number; target: string }> {
    const value = (this.root.getElementById(id) as HTMLTextAreaElement | null)?.value ?? "";
    return value.split(/\n|,/)
      .map((line) => line.trim())
      .filter(Boolean)
      .map((line) => {
        const [portRaw, targetRaw] = line.split("=");
        return { port: Number.parseInt(portRaw, 10), target: (targetRaw ?? "").trim() };
      })
      .filter((binding) => Number.isInteger(binding.port) && binding.port > 0 && binding.port <= 65535 && binding.target.length > 0);
  }

  private formatLayer4(bindings: Array<{ port: number; target: string }>): string {
    return bindings.map((binding) => `${binding.port}=${binding.target}`).join("\n");
  }

  private escape(value: string): string {
    return value.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
  }
}

customElements.define("tachyon-routing-panel", TachyonRoutingPanel);

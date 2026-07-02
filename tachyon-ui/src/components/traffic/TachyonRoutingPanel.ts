import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { el } from "../../utils/dom-safe";
import { resilientInvoke as invoke } from "../../utils/network";
import { t } from "../../utils/i18n";
import {
  getManifestOperatorConfig,
  listRoutePolicies,
  writeManifestOperatorConfig,
  writeRouteModelPolicy,
  writeRoutePolicy,
  writeRouteRequiresTee,
  type ManifestModelPolicyBinding,
  type ManifestOperatorConfig,
  type ManifestRoutePolicy,
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
  private routePolicies: ManifestRoutePolicy[] = [];

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
    try {
      this.routePolicies = await listRoutePolicies();
    } catch {
      this.routePolicies = [];
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
        const routePolicy = document.createElement("div");
        routePolicy.innerHTML = this.renderRoutePolicy(route.path);
        const volPanel = document.createElement("tachyon-volumes-panel") as HTMLElement;
        volPanel.setAttribute("route-path", route.path);
        const scopesPanel = document.createElement("tachyon-scopes-panel") as HTMLElement;
        scopesPanel.setAttribute("route-path", route.path);
        wrapper.appendChild(concPanel);
        wrapper.appendChild(routePolicy);
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
    tbody.querySelectorAll<HTMLFormElement>("[data-route-policy-form]").forEach((form) => {
      form.addEventListener("submit", (event) => {
        event.preventDefault();
        const routePath = form.dataset.routePolicyForm ?? "";
        if (routePath) void this.applyRoutePolicy(routePath, form);
      });
    });
    tbody.querySelectorAll<HTMLFormElement>("[data-model-policy-form]").forEach((form) => {
      form.addEventListener("submit", (event) => {
        event.preventDefault();
        const routePath = form.dataset.routePath ?? "";
        const modelAlias = form.dataset.modelAlias ?? "";
        if (routePath && modelAlias) void this.applyModelPolicy(routePath, modelAlias, form);
      });
    });
  }

  private renderRoutePolicy(routePath: string): string {
    const policy = this.routePolicies.find((item) => item.path === routePath);
    if (!policy) {
      return `
        <section class="mt-3 rounded-lg border border-slate-700 bg-slate-900/80 p-4">
          <h4 class="text-xs font-bold uppercase tracking-widest text-slate-400">Route policies</h4>
          <p class="mt-2 text-xs text-slate-500">Manifest route policy fields are unavailable until the manifest is loaded.</p>
        </section>
      `;
    }

    return `
      <section class="mt-3 space-y-4 rounded-lg border border-slate-700 bg-slate-900/80 p-4">
        <header>
          <h4 class="text-xs font-bold uppercase tracking-widest text-slate-400">Route policies</h4>
          <p class="mt-1 font-mono text-[11px] text-slate-500">routes[].distributed_rate_limit / resource_policy / adapter_id / shadow_target</p>
        </header>
        <form data-route-policy-form="${this.escape(policy.path)}" class="grid gap-3 md:grid-cols-4">
          <label class="text-xs text-slate-400">Rate threshold
            <input data-field="threshold" type="number" min="0" value="${policy.distributedRateLimit?.threshold ?? ""}" class="mt-1 w-full rounded border border-slate-600 bg-slate-950 px-2 py-1 text-xs text-slate-200">
          </label>
          <label class="text-xs text-slate-400">Rate window seconds
            <input data-field="window_seconds" type="number" min="0" value="${policy.distributedRateLimit?.window_seconds ?? ""}" class="mt-1 w-full rounded border border-slate-600 bg-slate-950 px-2 py-1 text-xs text-slate-200">
          </label>
          <label class="text-xs text-slate-400">Rate scope
            <input data-field="rate_scope" value="${this.escape(policy.distributedRateLimit?.scope ?? "")}" placeholder="tenant" class="mt-1 w-full rounded border border-slate-600 bg-slate-950 px-2 py-1 font-mono text-xs text-slate-200">
          </label>
          <label class="text-xs text-slate-400">Adapter ID
            <input data-field="adapter_id" value="${this.escape(policy.adapterId)}" placeholder="lora-tenant-a" class="mt-1 w-full rounded border border-slate-600 bg-slate-950 px-2 py-1 font-mono text-xs text-slate-200">
          </label>
          <label class="text-xs text-slate-400">VRAM MB
            <input data-field="vram_mb" type="number" min="0" value="${policy.resourcePolicy?.vram_mb ?? ""}" class="mt-1 w-full rounded border border-slate-600 bg-slate-950 px-2 py-1 text-xs text-slate-200">
          </label>
          <label class="text-xs text-slate-400">GPU affinity
            <input data-field="gpu_affinity" value="${this.escape(policy.resourcePolicy?.gpu_affinity ?? "")}" placeholder="gpu:0" class="mt-1 w-full rounded border border-slate-600 bg-slate-950 px-2 py-1 font-mono text-xs text-slate-200">
          </label>
          <label class="text-xs text-slate-400">Admission strategy
            <input data-field="admission_strategy" value="${this.escape(policy.resourcePolicy?.admission_strategy ?? "")}" placeholder="queue" class="mt-1 w-full rounded border border-slate-600 bg-slate-950 px-2 py-1 font-mono text-xs text-slate-200">
          </label>
          <label class="text-xs text-slate-400">Shadow target
            <input data-field="shadow_target" value="${this.escape(policy.shadowTarget)}" placeholder="/shadow-route" class="mt-1 w-full rounded border border-slate-600 bg-slate-950 px-2 py-1 font-mono text-xs text-slate-200">
          </label>
          <button type="submit" class="md:col-span-4 rounded border border-emerald-700/60 bg-emerald-800/30 px-3 py-1.5 text-xs font-semibold text-emerald-200 hover:bg-emerald-800/40">Save route policy</button>
        </form>
        ${this.renderModelPolicies(policy)}
      </section>
    `;
  }

  private renderModelPolicies(policy: ManifestRoutePolicy): string {
    if (policy.models.length === 0) {
      return `<p class="rounded border border-slate-800 bg-slate-950 px-3 py-2 text-xs text-slate-500">No models are bound to this route.</p>`;
    }
    return `
      <div class="space-y-2">
        <h5 class="text-[11px] font-semibold uppercase tracking-widest text-slate-500">Model policies</h5>
        ${policy.models.map((model) => this.renderModelPolicy(policy.path, model)).join("")}
      </div>
    `;
  }

  private renderModelPolicy(routePath: string, model: ManifestModelPolicyBinding): string {
    const env = Object.entries(model.env).map(([key, value]) => `${key}=${value}`).join("\n");
    return `
      <form data-model-policy-form data-route-path="${this.escape(routePath)}" data-model-alias="${this.escape(model.alias)}" class="grid gap-2 rounded border border-slate-700 bg-slate-800/50 p-3 md:grid-cols-5">
        <div class="md:col-span-5">
          <span class="font-mono text-xs text-cyan-200">${this.escape(model.alias || model.path)}</span>
          ${model.path ? `<span class="ml-2 font-mono text-[10px] text-slate-500">${this.escape(model.path)}</span>` : ""}
        </div>
        <label class="text-xs text-slate-400">QoS
          <input data-field="qos" value="${this.escape(model.qos)}" placeholder="gold" class="mt-1 w-full rounded border border-slate-600 bg-slate-950 px-2 py-1 font-mono text-xs text-slate-200">
        </label>
        <label class="text-xs text-slate-400">Min instances
          <input data-field="min_instances" type="number" min="0" value="${model.minInstances ?? ""}" class="mt-1 w-full rounded border border-slate-600 bg-slate-950 px-2 py-1 text-xs text-slate-200">
        </label>
        <label class="text-xs text-slate-400">Max concurrency
          <input data-field="max_concurrency" type="number" min="0" value="${model.maxConcurrency ?? ""}" class="mt-1 w-full rounded border border-slate-600 bg-slate-950 px-2 py-1 text-xs text-slate-200">
        </label>
        <label class="text-xs text-slate-400">Domains
          <input data-field="domains" value="${this.escape(model.domains.join(", "))}" placeholder="chat, embeddings" class="mt-1 w-full rounded border border-slate-600 bg-slate-950 px-2 py-1 font-mono text-xs text-slate-200">
        </label>
        <label class="text-xs text-slate-400">Env
          <textarea data-field="env" rows="1" placeholder="KEY=value" class="mt-1 w-full rounded border border-slate-600 bg-slate-950 px-2 py-1 font-mono text-xs text-slate-200">${this.escape(env)}</textarea>
        </label>
        <button type="submit" class="md:col-span-5 rounded border border-cyan-600/60 bg-cyan-700/30 px-3 py-1.5 text-xs text-cyan-200 hover:bg-cyan-700/40">Save model policy</button>
      </form>
    `;
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

  private async applyRoutePolicy(routePath: string, form: HTMLFormElement): Promise<void> {
    try {
      const response = await writeRoutePolicy(routePath, {
        distributedRateLimit: {
          threshold: this.optionalFormNumber(form, "threshold") ?? undefined,
          window_seconds: this.optionalFormNumber(form, "window_seconds") ?? undefined,
          scope: this.formValue(form, "rate_scope"),
        },
        resourcePolicy: {
          vram_mb: this.optionalFormNumber(form, "vram_mb") ?? undefined,
          gpu_affinity: this.formValue(form, "gpu_affinity"),
          admission_strategy: this.formValue(form, "admission_strategy"),
        },
        adapterId: this.formValue(form, "adapter_id"),
        shadowTarget: this.formValue(form, "shadow_target"),
      });
      this.routePolicies = await listRoutePolicies();
      this.showFeedback(response.success ? "success" : "error", response.message);
      this.populateSnapshotRows();
    } catch (error) {
      this.showFeedback("error", error instanceof Error ? error.message : String(error));
    }
  }

  private async applyModelPolicy(routePath: string, modelAlias: string, form: HTMLFormElement): Promise<void> {
    try {
      const response = await writeRouteModelPolicy(routePath, modelAlias, {
        qos: this.formValue(form, "qos"),
        minInstances: this.optionalFormNumber(form, "min_instances"),
        maxConcurrency: this.optionalFormNumber(form, "max_concurrency"),
        domains: this.formValue(form, "domains").split(","),
        env: this.parseEnv(this.formValue(form, "env")),
      });
      this.routePolicies = await listRoutePolicies();
      this.showFeedback(response.success ? "success" : "error", response.message);
      this.populateSnapshotRows();
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

  private formValue(form: HTMLFormElement, field: string): string {
    return (form.querySelector(`[data-field="${field}"]`) as HTMLInputElement | HTMLTextAreaElement | null)?.value.trim() ?? "";
  }

  private optionalFormNumber(form: HTMLFormElement, field: string): number | null {
    const value = this.formValue(form, field);
    if (!value) return null;
    const parsed = Number.parseInt(value, 10);
    return Number.isFinite(parsed) && parsed > 0 ? parsed : null;
  }

  private parseEnv(value: string): Record<string, string> {
    return Object.fromEntries(
      value
        .split(/\n|,/)
        .map((line) => line.trim())
        .filter(Boolean)
        .map((line) => {
          const [key, ...rest] = line.split("=");
          return [key.trim(), rest.join("=").trim()];
        })
        .filter(([key, val]) => key.length > 0 && val.length > 0),
    );
  }

  private formatLayer4(bindings: Array<{ port: number; target: string }>): string {
    return bindings.map((binding) => `${binding.port}=${binding.target}`).join("\n");
  }

  private escape(value: string): string {
    return value.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
  }
}

customElements.define("tachyon-routing-panel", TachyonRoutingPanel);

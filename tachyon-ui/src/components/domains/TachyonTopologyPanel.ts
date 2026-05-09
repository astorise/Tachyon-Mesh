import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { t } from "../../utils/i18n";
import stylesheetText from "../../style.css?inline";

export type TopologyNodeType =
  | "endpoint"
  | "system-faas"
  | "custom-wasm"
  | "llm"
  | "kv-cache"
  | "storage"
  | "message-broker"
  | "external-resource";

export type TopologyNode = {
  id: string;
  type: TopologyNodeType;
  label: string;
  x: number;
  y: number;
  data: Record<string, string>;
};

export type TopologyEdge = {
  id: string;
  from: string;
  to: string;
};

type NodeTheme = {
  card: string;
  badge: string;
  iconBg: string;
  glyph: string;
  domainHint: string;
};

const NODE_THEMES: Record<TopologyNodeType, NodeTheme> = {
  endpoint: {
    card: "bg-blue-900/40 border-blue-500/80 hover:border-blue-400 shadow-[0_0_15px_rgba(59,130,246,0.2)]",
    badge: "text-blue-300",
    iconBg: "bg-blue-500/20 text-blue-300",
    glyph: "🌐",
    domainHint: "routing",
  },
  "system-faas": {
    card: "bg-slate-800/60 border-slate-500/80 hover:border-slate-400 shadow-[0_0_15px_rgba(148,163,184,0.18)]",
    badge: "text-slate-300",
    iconBg: "bg-slate-500/20 text-slate-300",
    glyph: "🛡",
    domainHint: "system",
  },
  "custom-wasm": {
    card: "bg-cyan-900/40 border-cyan-400/80 hover:border-cyan-300 shadow-[0_0_15px_rgba(34,211,238,0.2)]",
    badge: "text-cyan-300",
    iconBg: "bg-cyan-500/20 text-cyan-300",
    glyph: "⟨/⟩",
    domainHint: "supply-chain",
  },
  llm: {
    card: "bg-fuchsia-900/40 border-fuchsia-500/80 hover:border-fuchsia-400 shadow-[0_0_15px_rgba(217,70,239,0.2)]",
    badge: "text-fuchsia-200",
    iconBg: "bg-fuchsia-500/20 text-fuchsia-200",
    glyph: "✦",
    domainHint: "ai",
  },
  "kv-cache": {
    card: "bg-amber-900/40 border-amber-500/80 hover:border-amber-400 shadow-[0_0_15px_rgba(245,158,11,0.2)]",
    badge: "text-amber-200",
    iconBg: "bg-amber-500/20 text-amber-200",
    glyph: "⚡",
    domainHint: "ai",
  },
  storage: {
    card: "bg-emerald-900/40 border-emerald-500/80 hover:border-emerald-400 shadow-[0_0_15px_rgba(16,185,129,0.2)]",
    badge: "text-emerald-200",
    iconBg: "bg-emerald-500/20 text-emerald-200",
    glyph: "▤",
    domainHint: "storage",
  },
  "message-broker": {
    card: "bg-indigo-900/40 border-indigo-500/80 hover:border-indigo-400 shadow-[0_0_15px_rgba(99,102,241,0.2)]",
    badge: "text-indigo-200",
    iconBg: "bg-indigo-500/20 text-indigo-200",
    glyph: "↹",
    domainHint: "data-events",
  },
  "external-resource": {
    card: "bg-rose-900/40 border-rose-500/80 hover:border-rose-400 shadow-[0_0_15px_rgba(244,63,94,0.2)]",
    badge: "text-rose-200",
    iconBg: "bg-rose-500/20 text-rose-200",
    glyph: "↗",
    domainHint: "supply-chain",
  },
};

const FALLBACK_THEME: NodeTheme = {
  card: "bg-slate-800/60 border-slate-600/60 hover:border-slate-500",
  badge: "text-slate-300",
  iconBg: "bg-slate-700/40 text-slate-300",
  glyph: "?",
  domainHint: "unknown",
};

function themeFor(type: string): NodeTheme {
  return (NODE_THEMES as Record<string, NodeTheme>)[type] ?? FALLBACK_THEME;
}

function badgeValueFor(node: TopologyNode): string {
  switch (node.type) {
    case "endpoint":
      return [node.data.protocol, node.data.port].filter(Boolean).join(":") || "—";
    case "system-faas":
      return node.data.component || "—";
    case "custom-wasm":
      return node.data.semver || "—";
    case "llm":
      return node.data.modelName || "—";
    case "kv-cache":
      return node.data.capacityGb ? `${node.data.capacityGb} GB` : "—";
    case "storage":
      return node.data.mountPath || "—";
    case "message-broker":
      return node.data.queueName || "—";
    case "external-resource":
      return node.data.targetUrl || "—";
    default:
      return "—";
  }
}

const canvasStylesheet = new CSSStyleSheet();
canvasStylesheet.replaceSync(stylesheetText);

export class TachyonTopologyCanvas extends HTMLElement {
  private readonly root: ShadowRoot;
  private nodes: TopologyNode[] = [];
  private edges: TopologyEdge[] = [];
  private selectedId: string | null = null;

  constructor() {
    super();
    this.root = this.attachShadow({ mode: "open" });
    this.root.adoptedStyleSheets = [canvasStylesheet];
  }

  setGraph(nodes: TopologyNode[], edges: TopologyEdge[]): void {
    this.nodes = nodes;
    this.edges = edges;
    this.render();
  }

  setSelected(id: string | null): void {
    this.selectedId = id;
    this.render();
  }

  serialize(): { nodes: Array<TopologyNode & { domainHint: string }>; edges: TopologyEdge[] } {
    return {
      nodes: this.nodes.map((node) => ({ ...node, domainHint: themeFor(node.type).domainHint })),
      edges: [...this.edges],
    };
  }

  connectedCallback(): void {
    this.render();
  }

  private render(): void {
    const width = 960;
    const height = 540;
    const edgeSvg = this.edges
      .map((edge) => {
        const from = this.nodes.find((n) => n.id === edge.from);
        const to = this.nodes.find((n) => n.id === edge.to);
        if (!from || !to) return "";
        const x1 = from.x + 128;
        const y1 = from.y + 36;
        const x2 = to.x + 128;
        const y2 = to.y + 36;
        return `<line x1="${x1}" y1="${y1}" x2="${x2}" y2="${y2}" stroke="rgba(34,211,238,0.5)" stroke-width="1.5" stroke-dasharray="4 3"/>`;
      })
      .join("");

    const nodeBlocks = this.nodes
      .map((node) => {
        const theme = themeFor(node.type);
        const isSelected = node.id === this.selectedId;
        const ring = isSelected ? "ring-2 ring-cyan-300" : "";
        return `
          <button data-node-id="${this.escape(node.id)}" type="button" class="absolute p-3 rounded-xl border-2 backdrop-blur-md w-64 cursor-pointer transition-colors text-left ${theme.card} ${ring}" style="left: ${node.x}px; top: ${node.y}px;">
            <div class="flex items-center gap-3 mb-2">
              <span class="inline-flex h-8 w-8 items-center justify-center rounded ${theme.iconBg} font-bold">${theme.glyph}</span>
              <div class="flex-1">
                <div class="text-[10px] uppercase tracking-widest text-slate-500">${this.escape(t(`topology.type.${node.type}`))}</div>
                <h4 class="${theme.badge} font-semibold text-sm truncate">${this.escape(node.label || node.id)}</h4>
              </div>
            </div>
            <div class="bg-slate-950/50 rounded px-2 py-1 flex justify-between items-center text-xs">
              <span class="text-slate-500">${this.escape(t(`topology.badge.${node.type}`))}</span>
              <span class="font-mono ${theme.badge} truncate ml-2">${this.escape(badgeValueFor(node))}</span>
            </div>
          </button>
        `;
      })
      .join("");

    this.root.innerHTML = `
      <div class="relative h-[540px] w-full overflow-hidden rounded-lg border border-slate-800 bg-slate-950/60 bg-[radial-gradient(circle_at_top,rgba(34,211,238,0.08),transparent_50%)]">
        <svg class="absolute inset-0 pointer-events-none" width="${width}" height="${height}">${edgeSvg}</svg>
        ${nodeBlocks}
      </div>
    `;

    this.root.querySelectorAll<HTMLButtonElement>("[data-node-id]").forEach((button) => {
      button.addEventListener("click", () => {
        const id = button.dataset.nodeId ?? "";
        this.dispatchEvent(
          new CustomEvent("topology:node-selected", {
            bubbles: true,
            composed: true,
            detail: { nodeId: id },
          }),
        );
      });
    });
  }

  private escape(value: string): string {
    return value
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#039;");
  }
}

customElements.define("tachyon-topology-canvas", TachyonTopologyCanvas);

const editorStylesheet = new CSSStyleSheet();
editorStylesheet.replaceSync(stylesheetText);

export class TachyonNodeEditor extends HTMLElement {
  private readonly root: ShadowRoot;
  private node: TopologyNode | null = null;

  constructor() {
    super();
    this.root = this.attachShadow({ mode: "open" });
    this.root.adoptedStyleSheets = [editorStylesheet];
  }

  setNode(node: TopologyNode | null): void {
    this.node = node ? { ...node, data: { ...node.data } } : null;
    this.render();
  }

  connectedCallback(): void {
    this.render();
  }

  private render(): void {
    if (!this.node) {
      this.root.innerHTML = `
        <aside class="hidden"></aside>
      `;
      return;
    }
    const theme = themeFor(this.node.type);
    this.root.innerHTML = `
      <aside class="fixed right-0 top-0 h-screen w-96 z-40 border-l border-slate-800 bg-slate-950/95 backdrop-blur-xl p-5 overflow-y-auto">
        <header class="flex items-center justify-between mb-5">
          <div>
            <div class="text-[10px] uppercase tracking-widest text-slate-500">${t(`topology.type.${this.node.type}`)}</div>
            <h3 class="${theme.badge} text-lg font-semibold">${this.escape(this.node.label || this.node.id)}</h3>
          </div>
          <button id="btn-close-editor" type="button" class="rounded border border-slate-700 bg-slate-800 px-2 py-1 text-xs text-slate-300 hover:bg-slate-700">${t("topology.editor.close")}</button>
        </header>

        <form id="node-form" class="space-y-3 text-sm">
          <label class="block text-xs uppercase tracking-widest text-cyan-500">${t("topology.editor.label")}
            <input id="node-label" type="text" value="${this.escape(this.node.label)}" class="mt-1 w-full rounded border border-slate-700 bg-slate-900 p-2 text-slate-200 outline-none focus:border-cyan-400" />
          </label>
          ${this.renderTypeFields()}
          <div class="flex justify-end gap-2 pt-3">
            <button type="submit" class="rounded border border-cyan-500/50 bg-cyan-500/15 px-3 py-2 text-xs font-medium text-cyan-200 hover:bg-cyan-500/25">${t("topology.editor.save")}</button>
          </div>
        </form>
      </aside>
    `;

    this.root.getElementById("btn-close-editor")?.addEventListener("click", () => {
      this.dispatchEvent(
        new CustomEvent("topology:editor-closed", {
          bubbles: true,
          composed: true,
        }),
      );
    });

    this.root.getElementById("node-form")?.addEventListener("submit", (event) => {
      event.preventDefault();
      this.commit();
    });
  }

  private renderTypeFields(): string {
    if (!this.node) return "";
    switch (this.node.type) {
      case "llm":
        return `
          <label class="block text-xs uppercase tracking-widest text-cyan-500">${t("topology.field.modelName")}
            <input id="field-modelName" type="text" value="${this.escape(this.node.data.modelName ?? "")}" class="mt-1 w-full rounded border border-slate-700 bg-slate-900 p-2 text-slate-200" />
          </label>
          <label class="block text-xs uppercase tracking-widest text-cyan-500">${t("topology.field.quantization")}
            <select id="field-quantization" class="mt-1 w-full rounded border border-slate-700 bg-slate-900 p-2 text-slate-200">
              ${this.option("quantization", "INT4")}
              ${this.option("quantization", "INT8")}
              ${this.option("quantization", "FP16")}
            </select>
          </label>
          <label class="block text-xs uppercase tracking-widest text-cyan-500">${t("topology.field.loraMode")}
            <select id="field-loraMode" class="mt-1 w-full rounded border border-slate-700 bg-slate-900 p-2 text-slate-200">
              ${this.option("loraMode", "dynamic")}
              ${this.option("loraMode", "static")}
            </select>
          </label>`;
      case "kv-cache":
        return `
          <label class="block text-xs uppercase tracking-widest text-cyan-500">${t("topology.field.capacityGb")}
            <input id="field-capacityGb" type="number" min="1" value="${this.escape(this.node.data.capacityGb ?? "32")}" class="mt-1 w-full rounded border border-slate-700 bg-slate-900 p-2 text-slate-200" />
          </label>
          <label class="block text-xs uppercase tracking-widest text-cyan-500">${t("topology.field.evictionPolicy")}
            <select id="field-evictionPolicy" class="mt-1 w-full rounded border border-slate-700 bg-slate-900 p-2 text-slate-200">
              ${this.option("evictionPolicy", "LRU")}
              ${this.option("evictionPolicy", "FIFO")}
            </select>
          </label>`;
      case "external-resource":
        return `
          <label class="block text-xs uppercase tracking-widest text-cyan-500">${t("topology.field.targetUrl")}
            <input id="field-targetUrl" type="url" value="${this.escape(this.node.data.targetUrl ?? "")}" class="mt-1 w-full rounded border border-slate-700 bg-slate-900 p-2 text-slate-200" />
          </label>
          <label class="block text-xs uppercase tracking-widest text-cyan-500">${t("topology.field.authType")}
            <select id="field-authType" class="mt-1 w-full rounded border border-slate-700 bg-slate-900 p-2 text-slate-200">
              ${this.option("authType", "None")}
              ${this.option("authType", "Bearer")}
              ${this.option("authType", "mTLS")}
            </select>
          </label>
          <label class="block text-xs uppercase tracking-widest text-cyan-500">${t("topology.field.timeoutMs")}
            <input id="field-timeoutMs" type="number" min="100" value="${this.escape(this.node.data.timeoutMs ?? "5000")}" class="mt-1 w-full rounded border border-slate-700 bg-slate-900 p-2 text-slate-200" />
          </label>`;
      case "custom-wasm":
        return `
          <label class="block text-xs uppercase tracking-widest text-cyan-500">${t("topology.field.capabilityName")}
            <input id="field-capabilityName" type="text" value="${this.escape(this.node.data.capabilityName ?? "")}" class="mt-1 w-full rounded border border-slate-700 bg-slate-900 p-2 text-slate-200" />
          </label>
          <label class="block text-xs uppercase tracking-widest text-cyan-500">${t("topology.field.semver")}
            <input id="field-semver" type="text" value="${this.escape(this.node.data.semver ?? "^1.0.0")}" class="mt-1 w-full rounded border border-slate-700 bg-slate-900 p-2 text-slate-200 font-mono" />
          </label>
          <label class="block text-xs uppercase tracking-widest text-cyan-500">${t("topology.field.assetSource")}
            <input id="field-assetSource" type="text" value="${this.escape(this.node.data.assetSource ?? "")}" placeholder="./assets/example.wasm" class="mt-1 w-full rounded border border-slate-700 bg-slate-900 p-2 text-slate-200 font-mono" />
          </label>`;
      case "endpoint":
        return `
          <label class="block text-xs uppercase tracking-widest text-cyan-500">${t("topology.field.protocol")}
            <select id="field-protocol" class="mt-1 w-full rounded border border-slate-700 bg-slate-900 p-2 text-slate-200">
              ${this.option("protocol", "HTTP")}
              ${this.option("protocol", "HTTPS")}
              ${this.option("protocol", "TCP")}
              ${this.option("protocol", "UDP")}
            </select>
          </label>
          <label class="block text-xs uppercase tracking-widest text-cyan-500">${t("topology.field.port")}
            <input id="field-port" type="number" value="${this.escape(this.node.data.port ?? "443")}" class="mt-1 w-full rounded border border-slate-700 bg-slate-900 p-2 text-slate-200 font-mono" />
          </label>`;
      case "storage":
        return `
          <label class="block text-xs uppercase tracking-widest text-cyan-500">${t("topology.field.mountPath")}
            <input id="field-mountPath" type="text" value="${this.escape(this.node.data.mountPath ?? "/data")}" class="mt-1 w-full rounded border border-slate-700 bg-slate-900 p-2 text-slate-200 font-mono" />
          </label>`;
      case "message-broker":
        return `
          <label class="block text-xs uppercase tracking-widest text-cyan-500">${t("topology.field.queueName")}
            <input id="field-queueName" type="text" value="${this.escape(this.node.data.queueName ?? "")}" class="mt-1 w-full rounded border border-slate-700 bg-slate-900 p-2 text-slate-200" />
          </label>`;
      case "system-faas":
        return `
          <label class="block text-xs uppercase tracking-widest text-cyan-500">${t("topology.field.component")}
            <input id="field-component" type="text" value="${this.escape(this.node.data.component ?? "")}" class="mt-1 w-full rounded border border-slate-700 bg-slate-900 p-2 text-slate-200" />
          </label>`;
      default:
        return "";
    }
  }

  private option(field: string, value: string): string {
    if (!this.node) return "";
    const selected = this.node.data[field] === value ? "selected" : "";
    return `<option value="${value}" ${selected}>${value}</option>`;
  }

  private commit(): void {
    if (!this.node) return;
    const labelInput = this.root.getElementById("node-label") as HTMLInputElement | null;
    if (labelInput) this.node.label = labelInput.value.trim();
    this.root.querySelectorAll<HTMLInputElement | HTMLSelectElement>("[id^='field-']").forEach((field) => {
      const key = field.id.replace(/^field-/, "");
      this.node!.data[key] = field.value.trim();
    });
    this.dispatchEvent(
      new CustomEvent("topology:node-updated", {
        bubbles: true,
        composed: true,
        detail: { node: { ...this.node, data: { ...this.node.data } } },
      }),
    );
  }

  private escape(value: string): string {
    return value
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#039;");
  }
}

customElements.define("tachyon-node-editor", TachyonNodeEditor);

const DEFAULT_NODES: TopologyNode[] = [
  { id: "edge-gateway", type: "endpoint", label: "Public HTTPS", x: 24, y: 24, data: { protocol: "HTTPS", port: "443" } },
  { id: "auth-faas", type: "system-faas", label: "AuthN", x: 320, y: 24, data: { component: "system-faas-authn" } },
  { id: "infer-llm", type: "llm", label: "Inference Hub", x: 624, y: 24, data: { modelName: "mistral-7b-instruct", quantization: "INT8", loraMode: "dynamic" } },
  { id: "kv-shared", type: "kv-cache", label: "Edge Cache", x: 320, y: 240, data: { capacityGb: "64", evictionPolicy: "LRU" } },
  { id: "stable-storage", type: "storage", label: "Mesh Volume", x: 24, y: 360, data: { mountPath: "/mnt/data" } },
  { id: "operator-broker", type: "message-broker", label: "Audit Stream", x: 624, y: 240, data: { queueName: "audit.events" } },
  { id: "ext-billing", type: "external-resource", label: "Billing API", x: 320, y: 420, data: { targetUrl: "https://billing.example.com", authType: "Bearer", timeoutMs: "5000" } },
  { id: "user-wasm", type: "custom-wasm", label: "Recommendation", x: 624, y: 420, data: { capabilityName: "guest-reco", semver: "^1.0.0", assetSource: "./assets/guest-reco.wasm" } },
];

const DEFAULT_EDGES: TopologyEdge[] = [
  { id: "e1", from: "edge-gateway", to: "auth-faas" },
  { id: "e2", from: "auth-faas", to: "infer-llm" },
  { id: "e3", from: "infer-llm", to: "kv-shared" },
  { id: "e4", from: "kv-shared", to: "stable-storage" },
  { id: "e5", from: "infer-llm", to: "operator-broker" },
  { id: "e6", from: "infer-llm", to: "ext-billing" },
  { id: "e7", from: "auth-faas", to: "user-wasm" },
];

export class TachyonTopologyPanel extends TachyonConfigDashboard {
  private nodes: TopologyNode[] = DEFAULT_NODES.map((node) => ({ ...node, data: { ...node.data } }));
  private edges: TopologyEdge[] = DEFAULT_EDGES.map((edge) => ({ ...edge }));
  private selectedId: string | null = null;
  private readonly onLanguageChanged = () => this.refresh();

  connectedCallback(): void {
    window.addEventListener("i18n:language-changed", this.onLanguageChanged);
    this.render();
    this.bindEvents();
    this.animateEntrance();
    this.pushGraphToCanvas();
  }

  disconnectedCallback(): void {
    window.removeEventListener("i18n:language-changed", this.onLanguageChanged);
  }

  private render(): void {
    this.renderTemplate(`
      <section class="p-6 space-y-6 text-slate-300">
        <header data-stagger-panel class="flex items-end justify-between gap-4 border-l-4 border-cyan-500 pl-4">
          <div>
            <h2 class="text-2xl font-bold text-slate-100">${t("topology.title")}</h2>
            <p class="text-sm font-mono text-slate-400">${t("topology.subtitle")}</p>
          </div>
          <button id="btn-build-bundle" type="button" class="rounded-md border border-cyan-500/40 bg-cyan-500/10 px-3 py-2 text-xs font-medium text-cyan-200 hover:bg-cyan-500/20">${t("topology.build-bundle")}</button>
        </header>

        <article data-stagger-panel>
          <tachyon-topology-canvas></tachyon-topology-canvas>
          <tachyon-node-editor></tachyon-node-editor>
        </article>

        <article data-stagger-panel class="rounded-lg border border-slate-800 bg-slate-900 p-4">
          <h3 class="mb-2 text-xs uppercase tracking-widest text-cyan-300">${t("topology.legend")}</h3>
          <div class="grid grid-cols-2 md:grid-cols-4 gap-2 text-xs">
            ${(Object.keys(NODE_THEMES) as TopologyNodeType[])
              .map((type) => {
                const theme = NODE_THEMES[type];
                return `<div class="flex items-center gap-2 rounded border border-slate-800 bg-slate-950/40 p-2"><span class="inline-flex h-6 w-6 items-center justify-center rounded ${theme.iconBg}">${theme.glyph}</span><span class="${theme.badge}">${this.escapeAttr(t(`topology.type.${type}`))}</span></div>`;
              })
              .join("")}
          </div>
        </article>

        <div id="feedback-zone" data-stagger-panel class="rounded-xl border border-slate-800 bg-slate-900 px-4 py-3 font-mono text-xs text-slate-400">${t("topology.feedback.empty")}</div>
      </section>
    `);
  }

  private bindEvents(): void {
    this.root.getElementById("btn-build-bundle")?.addEventListener("click", () => {
      const canvas = this.canvas();
      const detail = canvas?.serialize() ?? { nodes: [], edges: [] };
      this.dispatchEvent(
        new CustomEvent("topology:serialize", {
          bubbles: true,
          composed: true,
          detail,
        }),
      );
      this.showFeedback(
        "success",
        `${t("topology.feedback.serialized")} (${detail.nodes.length} ${t("topology.nodes")}, ${detail.edges.length} ${t("topology.edges")})`,
      );
    });

    const canvas = this.canvas();
    canvas?.addEventListener("topology:node-selected", (event) => {
      const id = (event as CustomEvent<{ nodeId: string }>).detail.nodeId;
      this.selectedId = id;
      const node = this.nodes.find((n) => n.id === id) ?? null;
      this.editor()?.setNode(node);
      canvas.setSelected(id);
    });

    const editor = this.editor();
    editor?.addEventListener("topology:node-updated", (event) => {
      const updated = (event as CustomEvent<{ node: TopologyNode }>).detail.node;
      const index = this.nodes.findIndex((n) => n.id === updated.id);
      if (index >= 0) {
        this.nodes[index] = { ...updated, data: { ...updated.data } };
        this.pushGraphToCanvas();
        this.showFeedback("success", t("topology.feedback.updated"));
      }
    });
    editor?.addEventListener("topology:editor-closed", () => {
      this.selectedId = null;
      this.editor()?.setNode(null);
      this.canvas()?.setSelected(null);
    });
  }

  private refresh(): void {
    this.render();
    this.bindEvents();
    this.pushGraphToCanvas();
  }

  private pushGraphToCanvas(): void {
    this.canvas()?.setGraph(
      this.nodes.map((node) => ({ ...node, data: { ...node.data } })),
      this.edges.map((edge) => ({ ...edge })),
    );
    this.canvas()?.setSelected(this.selectedId);
  }

  private canvas(): TachyonTopologyCanvas | null {
    return this.root.querySelector<TachyonTopologyCanvas>("tachyon-topology-canvas");
  }

  private editor(): TachyonNodeEditor | null {
    return this.root.querySelector<TachyonNodeEditor>("tachyon-node-editor");
  }

  private escapeAttr(value: string): string {
    return value.replace(/"/g, "&quot;");
  }
}

customElements.define("tachyon-topology-panel", TachyonTopologyPanel);

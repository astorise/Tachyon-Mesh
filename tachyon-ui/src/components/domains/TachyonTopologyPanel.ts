import { invoke } from "@tauri-apps/api/core";
import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { t } from "../../utils/i18n";
import stylesheetText from "../../style.css?inline";

type BundleConflict = { name: string; bundledVersion: string; clusterVersion: string };
type BundleApplyOutcome = {
  success: boolean;
  message: string;
  configVersion: number;
  conflicts: BundleConflict[];
  requiresResolution: boolean;
};

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

const VIRTUAL_W = 1920;
const VIRTUAL_H = 1080;
const CARD_W = 256;
const CARD_H = 80;
const BUBBLE_R = 24; // radius → 48px diameter

const canvasStylesheet = new CSSStyleSheet();
canvasStylesheet.replaceSync(stylesheetText);

export class TachyonTopologyCanvas extends HTMLElement {
  private readonly root: ShadowRoot;
  private nodes: TopologyNode[] = [];
  private edges: TopologyEdge[] = [];
  private selectedId: string | null = null;
  // Drag state (node repositioning)
  private dragState: { nodeId: string; offsetX: number; offsetY: number } | null = null;
  private didDrag = false;
  // Zoom / pan state
  private zoom = 1.0;
  private panX = 0;
  private panY = 0;
  private panState: { startX: number; startY: number; startPanX: number; startPanY: number } | null = null;
  // View mode
  private compactMode = false;

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
    // Lightweight update: just update ring classes without full re-render.
    this.root.querySelectorAll<HTMLButtonElement>("[data-node-id]").forEach((btn) => {
      const selected = btn.dataset.nodeId === id;
      if (selected) btn.classList.add("ring-2", "ring-cyan-300");
      else btn.classList.remove("ring-2", "ring-cyan-300");
    });
  }

  zoomIn(): void { this.adjustZoom(0.2); }
  zoomOut(): void { this.adjustZoom(-0.2); }

  resetView(): void {
    this.zoom = 1.0;
    this.panX = 0;
    this.panY = 0;
    this.applyViewTransform();
  }

  toggleCompact(): void {
    this.compactMode = !this.compactMode;
    this.render();
  }

  get isCompact(): boolean { return this.compactMode; }

  serialize(): { nodes: Array<TopologyNode & { domainHint: string }>; edges: TopologyEdge[] } {
    return {
      nodes: this.nodes.map((node) => ({ ...node, domainHint: themeFor(node.type).domainHint })),
      edges: [...this.edges],
    };
  }

  connectedCallback(): void {
    this.render();
  }

  private adjustZoom(delta: number): void {
    const outer = this.root.getElementById("canvas-outer");
    if (!outer) return;
    const rect = outer.getBoundingClientRect();
    const cx = rect.width / 2;
    const cy = rect.height / 2;
    const newZoom = Math.max(0.2, Math.min(4.0, this.zoom + delta));
    const factor = newZoom / this.zoom;
    this.panX = cx - factor * (cx - this.panX);
    this.panY = cy - factor * (cy - this.panY);
    this.zoom = newZoom;
    this.applyViewTransform();
  }

  private applyViewTransform(): void {
    const vp = this.root.getElementById("canvas-viewport");
    if (vp) {
      vp.style.transform = `translate(${this.panX}px, ${this.panY}px) scale(${this.zoom})`;
    }
  }

  private wireViewport(): void {
    const outer = this.root.getElementById("canvas-outer");
    if (!outer) return;

    outer.addEventListener("wheel", (e) => {
      e.preventDefault();
      const rect = outer.getBoundingClientRect();
      const cx = e.clientX - rect.left;
      const cy = e.clientY - rect.top;
      const delta = e.deltaY > 0 ? -0.12 : 0.12;
      const newZoom = Math.max(0.2, Math.min(4.0, this.zoom + delta));
      const factor = newZoom / this.zoom;
      this.panX = cx - factor * (cx - this.panX);
      this.panY = cy - factor * (cy - this.panY);
      this.zoom = newZoom;
      this.applyViewTransform();
    }, { passive: false });

    outer.addEventListener("pointerdown", (e) => {
      const target = e.target as Element;
      if (target.closest("[data-node-id]")) return;
      if (e.button !== 0 && e.button !== 1) return;
      e.preventDefault();
      outer.setPointerCapture(e.pointerId);
      outer.style.cursor = "grabbing";
      this.panState = { startX: e.clientX, startY: e.clientY, startPanX: this.panX, startPanY: this.panY };
    });

    outer.addEventListener("pointermove", (e) => {
      if (!this.panState) return;
      this.panX = this.panState.startPanX + (e.clientX - this.panState.startX);
      this.panY = this.panState.startPanY + (e.clientY - this.panState.startY);
      this.applyViewTransform();
    });

    outer.addEventListener("pointerup", () => {
      if (this.panState) {
        this.panState = null;
        outer.style.cursor = "grab";
      }
    });
  }

  private renderNode(node: TopologyNode): string {
    const theme = themeFor(node.type);
    const isSelected = node.id === this.selectedId;
    const ring = isSelected ? "ring-2 ring-cyan-300" : "";
    if (this.compactMode) {
      return `<button data-node-id="${this.escape(node.id)}" type="button"
        title="${this.escape(node.label || node.id)}"
        class="absolute flex items-center justify-center rounded-full border-2 touch-none select-none ${theme.card} ${ring}"
        style="left:${node.x}px;top:${node.y}px;width:48px;height:48px;cursor:grab;">
        <span style="font-size:18px;line-height:1">${theme.glyph}</span>
      </button>`;
    }
    return `<button data-node-id="${this.escape(node.id)}" type="button"
      class="absolute p-3 rounded-xl border-2 backdrop-blur-md w-64 touch-none select-none text-left ${theme.card} ${ring}"
      style="left:${node.x}px;top:${node.y}px;cursor:grab;">
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
    </button>`;
  }

  private render(): void {
    const nodeBlocks = this.nodes.map((n) => this.renderNode(n)).join("");
    const edgeSvg = this.buildEdgeSvg();
    const transform = `translate(${this.panX}px,${this.panY}px) scale(${this.zoom})`;

    this.root.innerHTML = `
      <div id="canvas-outer" class="relative h-[540px] w-full overflow-hidden rounded-lg border border-slate-800 bg-slate-950/60 bg-[radial-gradient(circle_at_top,rgba(34,211,238,0.08),transparent_50%)]" style="cursor:grab;">
        <div id="canvas-viewport" style="position:absolute;transform-origin:0 0;width:${VIRTUAL_W}px;height:${VIRTUAL_H}px;transform:${transform};">
          <svg class="absolute inset-0 pointer-events-none" width="${VIRTUAL_W}" height="${VIRTUAL_H}">${edgeSvg}</svg>
          ${nodeBlocks}
        </div>
      </div>
    `;

    this.wireViewport();
    this.wireFileDrop();
    this.wireNodeEvents();
  }

  private wireNodeEvents(): void {
    const viewport = this.root.getElementById("canvas-viewport");
    if (!viewport) return;

    this.root.querySelectorAll<HTMLButtonElement>("[data-node-id]").forEach((button) => {
      button.addEventListener("pointerdown", (event) => {
        this.didDrag = false;
        this.dragState = {
          nodeId: button.dataset.nodeId ?? "",
          offsetX: event.offsetX,
          offsetY: event.offsetY,
        };
        button.setPointerCapture(event.pointerId);
        button.style.cursor = "grabbing";
        button.style.zIndex = "10";
      });

      button.addEventListener("pointermove", (event) => {
        if (!this.dragState || this.dragState.nodeId !== (button.dataset.nodeId ?? "")) return;
        // Convert from screen coords to virtual canvas coords
        const newX = Math.max(0, Math.min(VIRTUAL_W - (this.compactMode ? 48 : CARD_W),
          (event.clientX - viewport.getBoundingClientRect().left) / this.zoom - this.dragState.offsetX));
        const newY = Math.max(0, Math.min(VIRTUAL_H - (this.compactMode ? 48 : CARD_H),
          (event.clientY - viewport.getBoundingClientRect().top) / this.zoom - this.dragState.offsetY));
        button.style.left = `${newX}px`;
        button.style.top = `${newY}px`;
        this.didDrag = true;
        this.updateEdgeSvgLive();
      });

      button.addEventListener("pointerup", (event) => {
        if (!this.dragState || this.dragState.nodeId !== (button.dataset.nodeId ?? "")) return;
        button.style.cursor = "grab";
        button.style.zIndex = "";
        if (this.didDrag) {
          const nodeW = this.compactMode ? 48 : CARD_W;
          const nodeH = this.compactMode ? 48 : CARD_H;
          const newX = Math.max(0, Math.min(VIRTUAL_W - nodeW,
            (event.clientX - viewport.getBoundingClientRect().left) / this.zoom - this.dragState.offsetX));
          const newY = Math.max(0, Math.min(VIRTUAL_H - nodeH,
            (event.clientY - viewport.getBoundingClientRect().top) / this.zoom - this.dragState.offsetY));
          const nodeId = this.dragState.nodeId;
          this.nodes = this.nodes.map((n) =>
            n.id === nodeId ? { ...n, x: Math.round(newX), y: Math.round(newY) } : n,
          );
          this.dispatchEvent(new CustomEvent("topology:node-moved", {
            bubbles: true, composed: true,
            detail: { nodeId, x: Math.round(newX), y: Math.round(newY) },
          }));
          this.updateEdgeSvgLive();
        }
        this.dragState = null;
      });

      button.addEventListener("click", (event) => {
        if (this.didDrag) { event.stopPropagation(); this.didDrag = false; return; }
        const id = button.dataset.nodeId ?? "";
        this.dispatchEvent(new CustomEvent("topology:node-selected", {
          bubbles: true, composed: true, detail: { nodeId: id },
        }));
      });
    });
  }

  private wireFileDrop(): void {
    const outer = this.root.getElementById("canvas-outer");
    if (!outer) return;

    outer.addEventListener("dragenter", (e) => {
      e.preventDefault();
      outer.style.borderColor = "rgba(34,211,238,0.8)";
      outer.style.boxShadow = "0 0 24px rgba(34,211,238,0.25)";
    });
    outer.addEventListener("dragover", (e) => { e.preventDefault(); });
    outer.addEventListener("dragleave", () => {
      outer.style.borderColor = "";
      outer.style.boxShadow = "";
    });
    outer.addEventListener("drop", (e) => {
      e.preventDefault();
      outer.style.borderColor = "";
      outer.style.boxShadow = "";
      const rect = outer.getBoundingClientRect();
      const files = Array.from((e as DragEvent).dataTransfer?.files ?? []);
      for (const file of files) {
        if (!file.name.endsWith(".wasm")) continue;
        // Convert screen coords to virtual canvas coords
        const vx = Math.max(0, Math.min(VIRTUAL_W - CARD_W,
          (e.clientX - rect.left - this.panX) / this.zoom - CARD_W / 2));
        const vy = Math.max(0, Math.min(VIRTUAL_H - CARD_H,
          (e.clientY - rect.top - this.panY) / this.zoom - CARD_H / 2));
        this.dispatchEvent(new CustomEvent("topology:wasm-dropped", {
          bubbles: true, composed: true,
          detail: {
            name: file.name.replace(/\.wasm$/, ""),
            path: (file as File & { path?: string }).path ?? file.name,
            x: Math.round(vx), y: Math.round(vy),
          },
        }));
      }
    });
  }

  private edgeCenter(node: TopologyNode): { x: number; y: number } {
    return this.compactMode
      ? { x: node.x + BUBBLE_R, y: node.y + BUBBLE_R }
      : { x: node.x + CARD_W / 2, y: node.y + CARD_H / 2 };
  }

  private buildEdgeSvg(): string {
    return this.edges
      .map((edge) => {
        const from = this.nodes.find((n) => n.id === edge.from);
        const to = this.nodes.find((n) => n.id === edge.to);
        if (!from || !to) return "";
        const fc = this.edgeCenter(from);
        const tc = this.edgeCenter(to);
        return `<line data-edge-id="${this.escape(edge.id)}" x1="${fc.x}" y1="${fc.y}" x2="${tc.x}" y2="${tc.y}" stroke="rgba(34,211,238,0.5)" stroke-width="1.5" stroke-dasharray="4 3"/>`;
      })
      .join("");
  }

  private updateEdgeSvgLive(): void {
    this.edges.forEach((edge) => {
      const line = this.root.querySelector<SVGLineElement>(`[data-edge-id="${edge.id}"]`);
      if (!line) return;
      const fromBtn = this.root.querySelector<HTMLButtonElement>(`[data-node-id="${edge.from}"]`);
      const toBtn = this.root.querySelector<HTMLButtonElement>(`[data-node-id="${edge.to}"]`);
      if (!fromBtn || !toBtn) return;
      const offset = this.compactMode ? BUBBLE_R : CARD_W / 2;
      const offsetY = this.compactMode ? BUBBLE_R : CARD_H / 2;
      line.setAttribute("x1", String((parseFloat(fromBtn.style.left) || 0) + offset));
      line.setAttribute("y1", String((parseFloat(fromBtn.style.top) || 0) + offsetY));
      line.setAttribute("x2", String((parseFloat(toBtn.style.left) || 0) + offset));
      line.setAttribute("y2", String((parseFloat(toBtn.style.top) || 0) + offsetY));
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
          <div class="flex items-center justify-between gap-2 pt-3">
            <button id="btn-delete-node" type="button" class="rounded border border-red-500/40 bg-red-500/10 px-3 py-2 text-xs font-medium text-red-300 hover:bg-red-500/20">${t("topology.editor.delete")}</button>
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

    this.root.getElementById("btn-delete-node")?.addEventListener("click", () => {
      if (!this.node) return;
      this.dispatchEvent(
        new CustomEvent("topology:node-delete", {
          bubbles: true,
          composed: true,
          detail: { nodeId: this.node.id },
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
  private nodes: TopologyNode[] = DEFAULT_NODES.map((n) => ({ ...n, data: { ...n.data } }));
  private edges: TopologyEdge[] = DEFAULT_EDGES.map((e) => ({ ...e }));
  private selectedId: string | null = null;
  private applying = false;
  private liveSource: string | null = null; // set when real data loaded
  private readonly onLanguageChanged = () => this.refresh();

  connectedCallback(): void {
    window.addEventListener("i18n:language-changed", this.onLanguageChanged);
    this.render();
    this.bindEvents();
    this.animateEntrance();
    this.pushGraphToCanvas();
    void this.loadLiveTopology();
  }

  disconnectedCallback(): void {
    window.removeEventListener("i18n:language-changed", this.onLanguageChanged);
  }

  private async loadLiveTopology(): Promise<void> {
    try {
      const graph = await invoke<{
        nodes: Array<{ id: string; nodeType: string; label: string; x: number; y: number; data: Record<string, string> }>;
        edges: Array<{ id: string; from: string; to: string }>;
        source: string;
        status: string;
      }>("get_topology_graph");

      if (graph.nodes.length === 0) return; // offline — keep sample

      this.nodes = graph.nodes.map((n) => ({
        id: n.id,
        type: (n.nodeType as TopologyNodeType) ?? "endpoint",
        label: n.label,
        x: n.x,
        y: n.y,
        data: { ...n.data },
      }));
      this.edges = graph.edges.map((e) => ({ ...e }));
      this.liveSource = graph.source;
      this.render();
      this.bindEvents();
      this.pushGraphToCanvas();
    } catch {
      // offline or Tauri not available — keep sample nodes
    }
  }

  private render(): void {
    const typeOptions = (Object.keys(NODE_THEMES) as TopologyNodeType[])
      .map((type) => `<option value="${type}">${t(`topology.type.${type}`)}</option>`)
      .join("");

    const sourceBanner = this.liveSource
      ? `<span class="ml-2 text-[10px] text-emerald-400 font-mono">${t("topology.live-banner")} ${this.escapeHtml(this.liveSource)}</span>`
      : `<span class="ml-2 text-[10px] text-amber-400/70 font-mono">${t("topology.offline-banner")}</span>`;

    const btnClass = "rounded border border-slate-700 bg-slate-800/60 px-2 py-1 text-xs text-slate-300 hover:bg-slate-700 transition-colors";

    this.renderTemplate(`
      <section class="p-6 space-y-4 text-slate-300">
        <header data-stagger-panel class="flex items-end justify-between gap-4 border-l-4 border-cyan-500 pl-4">
          <div>
            <h2 class="text-2xl font-bold text-slate-100">${t("topology.title")}${sourceBanner}</h2>
            <p class="text-sm font-mono text-slate-400">${t("topology.subtitle")}</p>
          </div>
          <button id="btn-apply-topology" type="button" ${this.applying ? "disabled" : ""}
            class="rounded-md border border-cyan-500/60 bg-cyan-600/20 px-4 py-2 text-xs font-bold text-cyan-200 hover:bg-cyan-600/30 disabled:opacity-50 disabled:cursor-not-allowed flex items-center gap-2">
            ${this.applying ? `<span class="inline-block h-3 w-3 animate-spin rounded-full border-2 border-cyan-300 border-t-transparent"></span>${t("topology.applying")}` : t("topology.apply-topology")}
          </button>
        </header>

        <article data-stagger-panel>
          <div class="mb-2 flex items-center gap-2 justify-end">
            <button id="btn-zoom-in"    type="button" class="${btnClass}" title="${t("topology.zoom-in")}">＋</button>
            <button id="btn-zoom-out"   type="button" class="${btnClass}" title="${t("topology.zoom-out")}">－</button>
            <button id="btn-zoom-reset" type="button" class="${btnClass}" title="${t("topology.zoom-reset")}">⊙</button>
            <button id="btn-compact"    type="button" class="${btnClass}" title="${t("topology.compact-mode")}">⊞</button>
          </div>
          <tachyon-topology-canvas></tachyon-topology-canvas>
          <tachyon-node-editor></tachyon-node-editor>
          <p class="mt-2 text-center text-[11px] text-slate-600 select-none">${t("topology.drop.hint")}</p>
        </article>

        <article data-stagger-panel class="rounded-lg border border-slate-800 bg-slate-900 p-4">
          <h3 class="mb-3 text-xs uppercase tracking-widest text-cyan-300">${t("topology.add.title")}</h3>
          <form id="add-node-form" class="flex flex-wrap items-end gap-3">
            <label class="block text-xs text-slate-400">${t("topology.add.type")}
              <select id="add-node-type" class="mt-1 rounded border border-slate-700 bg-slate-900 px-2 py-1.5 text-xs text-slate-200 outline-none focus:border-cyan-400">
                ${typeOptions}
              </select>
            </label>
            <label class="block text-xs text-slate-400">${t("topology.add.label")}
              <input id="add-node-label" type="text" class="mt-1 rounded border border-slate-700 bg-slate-900 px-2 py-1.5 text-xs text-slate-200 outline-none focus:border-cyan-400" style="width:180px;" />
            </label>
            <button type="submit" class="rounded border border-cyan-500/40 bg-cyan-500/10 px-3 py-1.5 text-xs font-medium text-cyan-200 hover:bg-cyan-500/20">${t("topology.add.submit")}</button>
          </form>
        </article>

        <article data-stagger-panel class="rounded-lg border border-slate-800 bg-slate-900 p-4">
          <h3 class="mb-2 text-xs uppercase tracking-widest text-cyan-300">${t("topology.legend")}</h3>
          <div class="grid grid-cols-2 md:grid-cols-4 gap-2 text-xs">
            ${(Object.keys(NODE_THEMES) as TopologyNodeType[])
              .map((type) => {
                const theme = NODE_THEMES[type];
                return `<div class="flex items-center gap-2 rounded border border-slate-800 bg-slate-950/40 p-2"><span class="inline-flex h-6 w-6 items-center justify-center rounded ${theme.iconBg}">${theme.glyph}</span><span class="${theme.badge}">${this.escapeHtml(t(`topology.type.${type}`))}</span></div>`;
              })
              .join("")}
          </div>
        </article>

        <div id="feedback-zone" data-stagger-panel class="rounded-xl border border-slate-800 bg-slate-900 px-4 py-3 font-mono text-xs text-slate-400">${t("topology.feedback.empty")}</div>
      </section>
    `);
  }

  private bindEvents(): void {
    this.root.getElementById("btn-apply-topology")?.addEventListener("click", () => {
      void this.applyTopology();
    });

    this.root.getElementById("btn-zoom-in")?.addEventListener("click", () => this.canvas()?.zoomIn());
    this.root.getElementById("btn-zoom-out")?.addEventListener("click", () => this.canvas()?.zoomOut());
    this.root.getElementById("btn-zoom-reset")?.addEventListener("click", () => this.canvas()?.resetView());
    this.root.getElementById("btn-compact")?.addEventListener("click", () => {
      const canvas = this.canvas();
      if (!canvas) return;
      canvas.toggleCompact();
      const btn = this.root.getElementById("btn-compact");
      if (btn) btn.textContent = canvas.isCompact ? "☰" : "⊞";
    });

    this.root.getElementById("add-node-form")?.addEventListener("submit", (event) => {
      event.preventDefault();
      this.addNode();
    });

    const canvas = this.canvas();
    canvas?.addEventListener("topology:wasm-dropped", (event) => {
      const { name, path, x, y } = (
        event as CustomEvent<{ name: string; path: string; x: number; y: number }>
      ).detail;
      const id = `wasm-${Date.now()}`;
      this.nodes.push({
        id,
        type: "custom-wasm",
        label: name,
        x,
        y,
        data: {
          capabilityName: name,
          semver: "^1.0.0",
          assetSource: path,
        },
      });
      this.pushGraphToCanvas();
      this.showFeedback("success", `${t("topology.feedback.wasm-dropped")} ${name}`);
    });
    canvas?.addEventListener("topology:node-selected", (event) => {
      const id = (event as CustomEvent<{ nodeId: string }>).detail.nodeId;
      this.selectedId = id;
      const node = this.nodes.find((n) => n.id === id) ?? null;
      this.editor()?.setNode(node);
      canvas.setSelected(id);
    });

    canvas?.addEventListener("topology:node-moved", (event) => {
      const { nodeId, x, y } = (event as CustomEvent<{ nodeId: string; x: number; y: number }>).detail;
      const index = this.nodes.findIndex((n) => n.id === nodeId);
      if (index >= 0) {
        this.nodes[index] = { ...this.nodes[index], x, y };
      }
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
    editor?.addEventListener("topology:node-delete", (event) => {
      const { nodeId } = (event as CustomEvent<{ nodeId: string }>).detail;
      this.nodes = this.nodes.filter((n) => n.id !== nodeId);
      this.edges = this.edges.filter((e) => e.from !== nodeId && e.to !== nodeId);
      this.selectedId = null;
      this.editor()?.setNode(null);
      this.pushGraphToCanvas();
      this.showFeedback("success", t("topology.feedback.deleted"));
    });
    editor?.addEventListener("topology:editor-closed", () => {
      this.selectedId = null;
      this.editor()?.setNode(null);
      this.canvas()?.setSelected(null);
    });
  }

  private async applyTopology(): Promise<void> {
    if (this.applying) return;
    this.applying = true;
    this.render();
    this.bindEvents();
    this.pushGraphToCanvas();
    this.showFeedback("success", t("topology.feedback.applying"));

    const dependencies = this.nodes
      .filter((n) => n.type === "custom-wasm" && n.data.assetSource)
      .map((n) => ({
        name: n.data.capabilityName || n.label || n.id,
        version: n.data.semver || "^1.0.0",
        source: n.data.assetSource || null,
      }));

    try {
      const result = await invoke<BundleApplyOutcome>("bundle_and_apply_manifest", {
        dependencies,
      });
      if (result.requiresResolution && result.conflicts.length > 0) {
        window.dispatchEvent(
          new CustomEvent("topology:conflict", {
            detail: { conflicts: result.conflicts },
          }),
        );
        this.showFeedback("error", t("topology.feedback.apply-conflict"));
      } else if (result.success) {
        this.showFeedback(
          "success",
          `${t("topology.feedback.apply-success")} ${result.configVersion}`,
        );
      } else {
        this.showFeedback("error", result.message);
      }
    } catch (error) {
      this.showFeedback("error", String(error));
    } finally {
      this.applying = false;
      this.render();
      this.bindEvents();
      this.pushGraphToCanvas();
    }
  }

  private addNode(): void {
    const typeEl = this.root.getElementById("add-node-type") as HTMLSelectElement | null;
    const labelEl = this.root.getElementById("add-node-label") as HTMLInputElement | null;
    const type = (typeEl?.value ?? "endpoint") as TopologyNodeType;
    const label = labelEl?.value.trim() || t(`topology.type.${type}`);
    const id = `node-${Date.now()}`;
    const x = 24 + Math.floor(Math.random() * 480);
    const y = 24 + Math.floor(Math.random() * 380);
    this.nodes.push({ id, type, label, x, y, data: {} });
    if (labelEl) labelEl.value = "";
    this.pushGraphToCanvas();
    this.showFeedback("success", t("topology.feedback.added"));
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

  private escapeHtml(value: string): string {
    return value
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#039;");
  }
}

customElements.define("tachyon-topology-panel", TachyonTopologyPanel);

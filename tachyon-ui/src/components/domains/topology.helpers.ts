import type { TopologyNode, TopologyNodeType } from "./TachyonTopologyPanel";

export type { TopologyNode, TopologyNodeType };

type NodeTheme = {
  card: string;
  badge: string;
  iconBg: string;
  glyph: string;
  domainHint: string;
};

export const NODE_TYPES: readonly TopologyNodeType[] = [
  "endpoint",
  "system-faas",
  "custom-wasm",
  "llm",
  "kv-cache",
  "storage",
  "message-broker",
  "external-resource",
];

const NODE_THEMES: Record<TopologyNodeType, NodeTheme> = {
  endpoint: { card: "bg-blue-900/40 border-blue-500/80", badge: "text-blue-300", iconBg: "bg-blue-500/20 text-blue-300", glyph: "🌐", domainHint: "routing" },
  "system-faas": { card: "bg-slate-800/60 border-slate-500/80", badge: "text-slate-300", iconBg: "bg-slate-500/20 text-slate-300", glyph: "🛡", domainHint: "system" },
  "custom-wasm": { card: "bg-cyan-900/40 border-cyan-400/80", badge: "text-cyan-300", iconBg: "bg-cyan-500/20 text-cyan-300", glyph: "⟨/⟩", domainHint: "supply-chain" },
  llm: { card: "bg-fuchsia-900/40 border-fuchsia-500/80", badge: "text-fuchsia-200", iconBg: "bg-fuchsia-500/20 text-fuchsia-200", glyph: "✦", domainHint: "ai" },
  "kv-cache": { card: "bg-amber-900/40 border-amber-500/80", badge: "text-amber-200", iconBg: "bg-amber-500/20 text-amber-200", glyph: "⚡", domainHint: "ai" },
  storage: { card: "bg-emerald-900/40 border-emerald-500/80", badge: "text-emerald-200", iconBg: "bg-emerald-500/20 text-emerald-200", glyph: "▤", domainHint: "storage" },
  "message-broker": { card: "bg-indigo-900/40 border-indigo-500/80", badge: "text-indigo-200", iconBg: "bg-indigo-500/20 text-indigo-200", glyph: "↹", domainHint: "data-events" },
  "external-resource": { card: "bg-rose-900/40 border-rose-500/80", badge: "text-rose-200", iconBg: "bg-rose-500/20 text-rose-200", glyph: "↗", domainHint: "supply-chain" },
};

const FALLBACK_THEME: NodeTheme = {
  card: "bg-slate-800/60 border-slate-600/60",
  badge: "text-slate-300",
  iconBg: "bg-slate-700/40 text-slate-300",
  glyph: "?",
  domainHint: "unknown",
};

export function themeFor(type: string): NodeTheme {
  return (NODE_THEMES as Record<string, NodeTheme>)[type] ?? FALLBACK_THEME;
}

export function badgeValueFor(node: TopologyNode): string {
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

export function serializeGraph(
  nodes: TopologyNode[],
  edges: Array<{ id: string; from: string; to: string }>,
): { nodes: Array<TopologyNode & { domainHint: string }>; edges: typeof edges } {
  return {
    nodes: nodes.map((node) => ({ ...node, domainHint: themeFor(node.type).domainHint })),
    edges: [...edges],
  };
}

export function filterGraphOnDelete(
  nodes: TopologyNode[],
  edges: Array<{ id: string; from: string; to: string }>,
  nodeId: string,
): { nodes: TopologyNode[]; edges: typeof edges } {
  return {
    nodes: nodes.filter((n) => n.id !== nodeId),
    edges: edges.filter((e) => e.from !== nodeId && e.to !== nodeId),
  };
}

export function clampPosition(
  x: number,
  y: number,
  canvasWidth: number,
  canvasHeight: number,
  nodeWidth = 256,
  nodeHeight = 80,
): { x: number; y: number } {
  return {
    x: Math.max(0, Math.min(canvasWidth - nodeWidth, x)),
    y: Math.max(0, Math.min(canvasHeight - nodeHeight, y)),
  };
}

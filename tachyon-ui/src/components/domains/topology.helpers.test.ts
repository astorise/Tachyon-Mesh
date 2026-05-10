import { describe, it, expect } from "vitest";
import {
  themeFor,
  badgeValueFor,
  serializeGraph,
  filterGraphOnDelete,
  clampPosition,
  NODE_TYPES,
} from "./topology.helpers";
import type { TopologyNode } from "./topology.helpers";

// ─────────────────────────────────────────────────────────────────────────────
// themeFor
// ─────────────────────────────────────────────────────────────────────────────

describe("themeFor", () => {
  it("returns a distinct non-fallback theme for each of the eight node types", () => {
    const seen = new Set<string>();
    for (const type of NODE_TYPES) {
      const theme = themeFor(type);
      expect(theme.glyph).not.toBe("?");
      seen.add(theme.glyph);
    }
    // All 8 types have unique glyphs.
    expect(seen.size).toBe(NODE_TYPES.length);
  });

  it("returns the fallback theme for an unknown type", () => {
    const theme = themeFor("unknown-type");
    expect(theme.glyph).toBe("?");
    expect(theme.domainHint).toBe("unknown");
  });

  it("llm theme has fuchsia palette and ai domain hint", () => {
    const theme = themeFor("llm");
    expect(theme.card).toContain("fuchsia");
    expect(theme.domainHint).toBe("ai");
  });

  it("endpoint theme has blue palette and routing domain hint", () => {
    const theme = themeFor("endpoint");
    expect(theme.card).toContain("blue");
    expect(theme.domainHint).toBe("routing");
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// badgeValueFor
// ─────────────────────────────────────────────────────────────────────────────

function node(type: TopologyNode["type"], data: Record<string, string>): TopologyNode {
  return { id: "n1", type, label: "Test", x: 0, y: 0, data };
}

describe("badgeValueFor", () => {
  it("endpoint: joins protocol and port with colon", () => {
    expect(badgeValueFor(node("endpoint", { protocol: "HTTPS", port: "443" }))).toBe("HTTPS:443");
  });

  it("endpoint: returns dash when both are empty", () => {
    expect(badgeValueFor(node("endpoint", {}))).toBe("—");
  });

  it("llm: returns model name", () => {
    expect(badgeValueFor(node("llm", { modelName: "mistral-7b" }))).toBe("mistral-7b");
  });

  it("kv-cache: appends GB suffix", () => {
    expect(badgeValueFor(node("kv-cache", { capacityGb: "64" }))).toBe("64 GB");
  });

  it("kv-cache: returns dash when capacityGb is empty", () => {
    expect(badgeValueFor(node("kv-cache", {}))).toBe("—");
  });

  it("storage: returns mount path", () => {
    expect(badgeValueFor(node("storage", { mountPath: "/mnt/data" }))).toBe("/mnt/data");
  });

  it("external-resource: returns target URL", () => {
    expect(
      badgeValueFor(node("external-resource", { targetUrl: "https://api.example.com" })),
    ).toBe("https://api.example.com");
  });

  it("custom-wasm: returns semver string", () => {
    expect(badgeValueFor(node("custom-wasm", { semver: "^1.2.0" }))).toBe("^1.2.0");
  });

  it("message-broker: returns queue name", () => {
    expect(badgeValueFor(node("message-broker", { queueName: "audit.events" }))).toBe(
      "audit.events",
    );
  });

  it("system-faas: returns component name", () => {
    expect(badgeValueFor(node("system-faas", { component: "system-faas-authn" }))).toBe(
      "system-faas-authn",
    );
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// serializeGraph
// ─────────────────────────────────────────────────────────────────────────────

describe("serializeGraph", () => {
  const nodes: TopologyNode[] = [
    node("llm", { modelName: "mistral" }),
    node("storage", { mountPath: "/data" }),
  ];
  const edges = [{ id: "e1", from: "n1", to: "n1" }];

  it("attaches domainHint to each node", () => {
    const result = serializeGraph(nodes, edges);
    expect(result.nodes[0].domainHint).toBe("ai");
    expect(result.nodes[1].domainHint).toBe("storage");
  });

  it("copies edges without mutation", () => {
    const result = serializeGraph(nodes, edges);
    expect(result.edges).toEqual(edges);
    expect(result.edges).not.toBe(edges);
  });

  it("preserves node data", () => {
    const result = serializeGraph(nodes, edges);
    expect(result.nodes[0].data.modelName).toBe("mistral");
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// filterGraphOnDelete
// ─────────────────────────────────────────────────────────────────────────────

describe("filterGraphOnDelete", () => {
  const a: TopologyNode = { id: "a", type: "endpoint", label: "A", x: 0, y: 0, data: {} };
  const b: TopologyNode = { id: "b", type: "llm", label: "B", x: 100, y: 0, data: {} };
  const c: TopologyNode = { id: "c", type: "storage", label: "C", x: 200, y: 0, data: {} };
  const edges = [
    { id: "e1", from: "a", to: "b" },
    { id: "e2", from: "b", to: "c" },
    { id: "e3", from: "a", to: "c" },
  ];

  it("removes the target node", () => {
    const result = filterGraphOnDelete([a, b, c], edges, "b");
    expect(result.nodes.map((n) => n.id)).toEqual(["a", "c"]);
  });

  it("removes all edges that reference the deleted node", () => {
    const result = filterGraphOnDelete([a, b, c], edges, "b");
    expect(result.edges).toEqual([{ id: "e3", from: "a", to: "c" }]);
  });

  it("leaves other nodes and edges untouched when deleting a leaf", () => {
    const result = filterGraphOnDelete([a, b, c], edges, "a");
    expect(result.nodes.map((n) => n.id)).toEqual(["b", "c"]);
    expect(result.edges).toEqual([{ id: "e2", from: "b", to: "c" }]);
  });

  it("returns empty arrays when the only node is deleted", () => {
    const result = filterGraphOnDelete([a], [{ id: "e1", from: "a", to: "a" }], "a");
    expect(result.nodes).toHaveLength(0);
    expect(result.edges).toHaveLength(0);
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// clampPosition
// ─────────────────────────────────────────────────────────────────────────────

describe("clampPosition", () => {
  it("returns the position unchanged when inside bounds", () => {
    const result = clampPosition(100, 200, 960, 540);
    expect(result).toEqual({ x: 100, y: 200 });
  });

  it("clamps x to 0 when negative", () => {
    const result = clampPosition(-50, 200, 960, 540);
    expect(result.x).toBe(0);
  });

  it("clamps x so the node stays within canvas width", () => {
    const result = clampPosition(900, 200, 960, 540);
    expect(result.x).toBe(960 - 256);
  });

  it("clamps y to 0 when negative", () => {
    const result = clampPosition(100, -10, 960, 540);
    expect(result.y).toBe(0);
  });

  it("clamps y so the node stays within canvas height", () => {
    const result = clampPosition(100, 520, 960, 540);
    expect(result.y).toBe(540 - 80);
  });

  it("respects custom node dimensions", () => {
    const result = clampPosition(900, 500, 960, 540, 100, 50);
    expect(result.x).toBe(860);
    expect(result.y).toBe(490);
  });
});

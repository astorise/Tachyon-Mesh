import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { resilientInvoke } from "../../utils/network";
import * as scopesController from "../../controllers/scopesController";

vi.mock("../../utils/network", () => ({
  resilientInvoke: vi.fn(),
}));

vi.mock("../../controllers/scopesController", () => ({
  readRouteScopes: vi.fn(),
  writeRouteScopes: vi.fn(),
  readScopeMetrics: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import "./TachyonObservabilityPanel";

function makeMetrics(overrides: Partial<{
  source: string;
  errorRate: number;
  p50LatencyMs: number;
  p99LatencyMs: number;
  queueDepth: number;
  scopeDenialTotal: number;
}> = {}) {
  return {
    source: "test-node",
    errorRate: 0,
    p50LatencyMs: 1.0,
    p99LatencyMs: 2.0,
    queueDepth: 0,
    scopeDenialTotal: 0,
    ...overrides,
  };
}

// resilientInvoke must return typed arrays for tail_logs and get_shadow_diffs;
// returning a metrics object causes populateObservability() to call .map() on it and throw.
function mockInvokeForObservability(metricsOverrides: Parameters<typeof makeMetrics>[0] = {}) {
  vi.mocked(resilientInvoke).mockImplementation(async (cmd: string) => {
    if (cmd === "get_metrics") return makeMetrics(metricsOverrides);
    if (cmd === "tail_logs") return [];
    if (cmd === "get_shadow_diffs") return [];
    return null;
  });
}

// connectedCallback runs several awaits (3× resilientInvoke + RT promise + readScopeMetrics +
// DOM update) before the scope widget is fully settled. 10 microtask ticks cover both 1-step
// and 2-step async-await implementations across V8 versions.
async function flushAll(): Promise<void> {
  for (let i = 0; i < 10; i++) await Promise.resolve();
}

describe("TachyonObservabilityPanel — Scope Denials widget (task 5.5)", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
    vi.useFakeTimers();
    vi.mocked(resilientInvoke).mockReset();
    vi.mocked(scopesController.readScopeMetrics).mockReset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("shows zero-state message when scopeDenialTotal is 0", async () => {
    mockInvokeForObservability();
    vi.mocked(scopesController.readScopeMetrics).mockResolvedValue({ scopeDenialTotal: 0 });

    const panel = document.createElement("tachyon-observability-panel");
    document.body.appendChild(panel);
    await flushAll();

    const sr = panel.shadowRoot!;
    const emptyEl = sr.getElementById("scope-denial-empty");
    const counterEl = sr.getElementById("scope-denial-counter");

    expect(emptyEl?.classList.contains("hidden")).toBe(false);
    expect(counterEl?.classList.contains("hidden")).toBe(true);
    expect(sr.textContent).toContain("No scope denials recorded");
  });

  it("shows the denial counter when scopeDenialTotal > 0", async () => {
    mockInvokeForObservability();
    vi.mocked(scopesController.readScopeMetrics).mockResolvedValue({ scopeDenialTotal: 42 });

    const panel = document.createElement("tachyon-observability-panel");
    document.body.appendChild(panel);
    await flushAll();

    const sr = panel.shadowRoot!;
    const emptyEl = sr.getElementById("scope-denial-empty");
    const counterEl = sr.getElementById("scope-denial-counter");
    const countEl = sr.getElementById("scope-denial-count");

    expect(counterEl?.classList.contains("hidden")).toBe(false);
    expect(emptyEl?.classList.contains("hidden")).toBe(true);
    expect(countEl?.textContent?.trim()).toBe("42");
  });

  it("updates the counter on the 30-second refresh interval", async () => {
    mockInvokeForObservability();
    vi.mocked(scopesController.readScopeMetrics)
      .mockResolvedValueOnce({ scopeDenialTotal: 0 })
      .mockResolvedValueOnce({ scopeDenialTotal: 7 });

    const panel = document.createElement("tachyon-observability-panel");
    document.body.appendChild(panel);
    await flushAll();

    const sr = panel.shadowRoot!;
    expect(sr.getElementById("scope-denial-empty")?.classList.contains("hidden")).toBe(false);

    // Advance the 30-second interval timer and flush
    await vi.advanceTimersByTimeAsync(30_000);
    await flushAll();

    const countEl = sr.getElementById("scope-denial-count");
    expect(countEl?.textContent?.trim()).toBe("7");
    expect(sr.getElementById("scope-denial-counter")?.classList.contains("hidden")).toBe(false);
  });

  it("clears the interval on disconnectedCallback", async () => {
    mockInvokeForObservability();
    vi.mocked(scopesController.readScopeMetrics).mockResolvedValue({ scopeDenialTotal: 0 });

    const panel = document.createElement("tachyon-observability-panel");
    document.body.appendChild(panel);
    await flushAll();

    const callCountBefore = vi.mocked(scopesController.readScopeMetrics).mock.calls.length;
    panel.remove();

    await vi.advanceTimersByTimeAsync(60_000);
    await flushAll();

    // No additional readScopeMetrics calls after removal
    expect(vi.mocked(scopesController.readScopeMetrics).mock.calls.length).toBe(callCountBefore);
  });
});

describe("TachyonObservabilityPanel — E2E scope widget integration (task 6.4)", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
    vi.useFakeTimers();
    vi.mocked(resilientInvoke).mockReset();
    vi.mocked(scopesController.readScopeMetrics).mockReset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("prometheus note is shown alongside a non-zero counter", async () => {
    mockInvokeForObservability();
    vi.mocked(scopesController.readScopeMetrics).mockResolvedValue({ scopeDenialTotal: 5 });

    const panel = document.createElement("tachyon-observability-panel");
    document.body.appendChild(panel);
    await flushAll();

    const sr = panel.shadowRoot!;
    expect(sr.textContent).toContain("faas_scope_denials_total");
    expect(sr.textContent).toContain("lifetime denials");
  });

  it("configure-scopes link dispatches app:navigate event", async () => {
    mockInvokeForObservability();
    vi.mocked(scopesController.readScopeMetrics).mockResolvedValue({ scopeDenialTotal: 3 });

    const panel = document.createElement("tachyon-observability-panel");
    document.body.appendChild(panel);
    await flushAll();

    const sr = panel.shadowRoot!;
    const link = sr.getElementById("link-configure-scopes") as HTMLAnchorElement | null;
    expect(link).not.toBeNull();

    const events: CustomEvent[] = [];
    window.addEventListener("app:navigate", (e) => events.push(e as CustomEvent));

    link?.click();
    expect(events).toHaveLength(1);
    expect(events[0].detail).toEqual({ panel: "routing" });

    window.removeEventListener("app:navigate", (e) => events.push(e as CustomEvent));
  });

  it("readScopeMetrics is called once at init and once per 30s interval", async () => {
    mockInvokeForObservability();
    vi.mocked(scopesController.readScopeMetrics).mockResolvedValue({ scopeDenialTotal: 0 });

    const panel = document.createElement("tachyon-observability-panel");
    document.body.appendChild(panel);
    await flushAll();

    const initialCalls = vi.mocked(scopesController.readScopeMetrics).mock.calls.length;
    expect(initialCalls).toBe(1);

    await vi.advanceTimersByTimeAsync(30_000);
    await flushAll();
    expect(vi.mocked(scopesController.readScopeMetrics).mock.calls.length).toBe(2);

    await vi.advanceTimersByTimeAsync(30_000);
    await flushAll();
    expect(vi.mocked(scopesController.readScopeMetrics).mock.calls.length).toBe(3);
  });
});

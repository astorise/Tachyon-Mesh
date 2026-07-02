import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { t } from "../../utils/i18n";
import {
  listRouteResiliencyPolicies,
  writeRouteResiliency,
  type ManifestRouteResiliency,
} from "../../controllers/manifestConfigController";

export class TachyonResiliencePanel extends TachyonConfigDashboard {
  private readonly onLanguageChanged = () => { this.render(); this.bindForm(); };
  private routes: ManifestRouteResiliency[] = [];
  private selectedRoute = "";

  async connectedCallback(): Promise<void> {
    window.addEventListener("i18n:language-changed", this.onLanguageChanged);
    this.render();
    this.bindForm();
    this.animateEntrance();
    try {
      this.routes = await listRouteResiliencyPolicies();
      this.selectedRoute = this.routes[0]?.path ?? "";
      this.render();
      this.bindForm();
      this.syncSelectedRouteFields();
    } catch (error) {
      this.showFeedback("error", error instanceof Error ? error.message : String(error));
    }
  }

  disconnectedCallback(): void {
    window.removeEventListener("i18n:language-changed", this.onLanguageChanged);
  }

  private render(): void {
    this.renderTemplate(`
      <section class="p-6 space-y-6 text-slate-300">
        <div data-stagger-panel class="border-l-4 border-cyan-500 pl-4">
          <div class="flex items-baseline gap-2">
            <h2 class="text-2xl font-bold text-slate-100">${t("resilience.title")}</h2>
          </div>
          <p class="text-slate-400 text-sm font-mono">Manifest: routes[].resiliency</p>
        </div>

        <form class="space-y-6">
          <div data-stagger-panel class="grid grid-cols-1 md:grid-cols-4 gap-4 bg-slate-800/40 p-6 rounded-lg border border-slate-700">
            <label class="block text-xs uppercase tracking-widest text-cyan-500">Route
              <select id="route-path" class="mt-1 w-full bg-slate-900 border border-slate-600 p-2 rounded text-slate-200 outline-none focus:border-cyan-400 transition-colors">
                ${this.routes.map((route) => `
                  <option value="${this.escape(route.path)}"${route.path === this.selectedRoute ? " selected" : ""}>${this.escape(route.name || route.path)}</option>
                `).join("")}
              </select>
            </label>
            <label class="block text-xs uppercase tracking-widest text-cyan-500">${t("resilience.field.timeout")}
              <input id="timeout-ms" type="number" min="1" value="1500" class="mt-1 w-full bg-slate-900 border border-slate-600 p-2 rounded text-slate-200 outline-none focus:border-cyan-400 transition-colors">
            </label>
            <label class="block text-xs uppercase tracking-widest text-cyan-500">${t("resilience.field.retry")}
              <input id="retry-count" type="number" min="0" value="2" class="mt-1 w-full bg-slate-900 border border-slate-600 p-2 rounded text-slate-200 outline-none focus:border-cyan-400 transition-colors">
            </label>
            <label class="block text-xs uppercase tracking-widest text-cyan-500">Retry status codes
              <input id="retry-on" type="text" value="502,503,504" class="mt-1 w-full bg-slate-900 border border-slate-600 p-2 rounded text-slate-200 outline-none focus:border-cyan-400 transition-colors">
            </label>
          </div>

          <button id="apply-btn" ${this.routes.length === 0 ? "disabled" : ""} class="bg-cyan-600 hover:bg-cyan-500 disabled:cursor-not-allowed disabled:opacity-50 text-slate-900 font-black py-3 px-8 rounded-sm uppercase tracking-tighter transition-all">
            ${t("resilience.button")}
          </button>
        </form>

        <div id="feedback-zone" data-stagger-panel class="mt-4 rounded-xl border border-slate-800 bg-slate-900 px-4 py-3 font-mono text-xs text-slate-400">${t("resilience.feedback.empty")}</div>
      </section>
    `);
  }

  private bindForm(): void {
    this.root.querySelector("form")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.applyPolicy();
    });
    this.root.getElementById("route-path")?.addEventListener("change", () => {
      this.selectedRoute = this.value("route-path", this.selectedRoute);
      this.syncSelectedRouteFields();
    });
  }

  private async applyPolicy(): Promise<void> {
    try {
      const routePath = this.value("route-path", "");
      if (!routePath) {
        this.showFeedback("error", "No manifest route is available for resiliency policy.");
        return;
      }
      const response = await writeRouteResiliency(routePath, {
        timeout_ms: this.numberValue("timeout-ms", 1500),
        retry_policy: {
          max_retries: this.numberValue("retry-count", 2),
          retry_on: this.numberList("retry-on"),
        },
      });
      this.routes = await listRouteResiliencyPolicies();
      this.showFeedback(response.success ? "success" : "error", response.message);
    } catch (error) {
      this.showFeedback("error", error instanceof Error ? error.message : String(error));
    }
  }

  private numberValue(id: string, fallback: number): number {
    const input = this.root.getElementById(id) as HTMLInputElement | null;
    if (!input) return fallback;
    const raw = input.valueAsNumber;
    return Number.isFinite(raw) ? raw : fallback;
  }

  private setNumber(id: string, value: unknown): void {
    const el = this.root.getElementById(id) as HTMLInputElement | null;
    if (el && (typeof value === "number" || typeof value === "string")) el.value = String(value);
  }

  private numberList(id: string): number[] {
    return this.value(id, "502,503,504")
      .split(",")
      .map((item) => Number.parseInt(item.trim(), 10))
      .filter((status) => Number.isInteger(status) && status >= 100 && status <= 599);
  }

  private syncSelectedRouteFields(): void {
    const route = this.routes.find((item) => item.path === this.selectedRoute);
    this.setNumber("timeout-ms", route?.resiliency?.timeout_ms ?? 1500);
    this.setNumber("retry-count", route?.resiliency?.retry_policy?.max_retries ?? 2);
    const retryOn = route?.resiliency?.retry_policy?.retry_on ?? [502, 503, 504];
    const retryInput = this.root.getElementById("retry-on") as HTMLInputElement | null;
    if (retryInput) retryInput.value = retryOn.join(",");
  }

  private value(id: string, fallback: string): string {
    const value = (this.root.getElementById(id) as HTMLInputElement | HTMLSelectElement | null)?.value.trim();
    return value ? value : fallback;
  }

  private escape(value: string): string {
    return value.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
  }
}

customElements.define("tachyon-resilience-panel", TachyonResiliencePanel);

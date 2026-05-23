import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { readRouteScopes, writeRouteScopes } from "../../controllers/scopesController";
import { isValidGlob, isValidRoutingTuple } from "../../utils/glob-validate";
import { SCOPE_CATEGORIES, type ScopeCategory, type ScopesMap } from "../../types/scopes";

export class TachyonScopesPanel extends TachyonConfigDashboard {
  private routePath = "";
  private scopes: ScopesMap = {};
  private allowAll = true;
  private dirty = false;
  private validationError = "";

  static get observedAttributes() {
    return ["route-path"];
  }

  attributeChangedCallback(name: string, _old: string, value: string): void {
    if (name === "route-path") {
      this.routePath = value;
    }
  }

  async connectedCallback(): Promise<void> {
    if (this.hasAttribute("route-path")) {
      this.routePath = this.getAttribute("route-path") ?? "";
    }
    this.render();
    await this.load();
  }

  private async load(): Promise<void> {
    await this.withLoadingState(async () => {
      const result = await readRouteScopes(this.routePath);
      this.scopes = result.scopes;
      this.allowAll = result.allowAll;
      this.dirty = false;
      this.render();
      this.bindEvents();
    });
  }

  private render(): void {
    const hasScopes = !this.allowAll && Object.keys(this.scopes).length > 0;
    const chipCount = hasScopes
      ? Object.values(this.scopes).reduce((n, arr) => n + (arr?.length ?? 0), 0)
      : 0;

    this.renderTemplate(`
      <section class="mt-3 rounded-lg border ${this.allowAll ? "border-amber-500/40" : "border-slate-700"} bg-slate-900/80 p-4 space-y-3">
        <header class="flex items-center justify-between gap-3">
          <h4 class="text-xs font-bold uppercase tracking-widest text-slate-400">Scopes</h4>
          <div class="flex items-center gap-2">
            ${this.allowAll
              ? `<span class="rounded-full bg-amber-500/20 border border-amber-500/40 px-2 py-0.5 text-[10px] font-semibold text-amber-300 uppercase tracking-widest">ALLOW ALL</span>`
              : chipCount > 0
                ? `<span class="text-[10px] text-slate-500">${chipCount} pattern${chipCount !== 1 ? "s" : ""}</span>`
                : ""}
            ${this.dirty
              ? `<button id="btn-save-scopes" class="rounded border border-emerald-600/50 bg-emerald-700/20 px-2 py-1 text-xs text-emerald-300 hover:bg-emerald-700/30">Save scopes</button>`
              : ""}
          </div>
        </header>

        ${this.allowAll
          ? `<p class="text-[11px] text-amber-400/80 italic">This route grants all WIT imports. Configure explicit scopes to restrict access.</p>`
          : ""}

        ${this.validationError
          ? `<p class="text-[11px] text-red-400 px-2 py-1 bg-red-900/20 rounded">${this.escHtml(this.validationError)}</p>`
          : ""}

        <div id="scopes-categories" class="space-y-2">
          ${SCOPE_CATEGORIES.map((cat) => this.renderCategory(cat)).join("")}
        </div>

        <div id="feedback-zone" class="hidden"></div>
      </section>
    `);
  }

  private renderCategory(cat: ScopeCategory): string {
    const patterns = this.scopes[cat] ?? [];
    const isEmpty = patterns.length === 0;
    return `
      <div class="rounded border border-slate-800 bg-slate-950/40 p-2" data-category="${cat}">
        <div class="flex items-center justify-between mb-1">
          <span class="text-[10px] uppercase tracking-widest font-mono ${isEmpty ? "text-slate-600" : "text-cyan-400"}">${cat}</span>
          ${isEmpty
            ? `<span class="text-[10px] text-slate-600 italic">not granted</span>`
            : `<button class="btn-remove-category text-[10px] text-slate-500 hover:text-red-400" data-category="${cat}">clear</button>`}
        </div>
        ${patterns.length > 0
          ? `<div class="flex flex-wrap gap-1 mb-1">
              ${patterns.map((p, i) => `
                <span class="inline-flex items-center gap-1 rounded-full border border-slate-700 bg-slate-800 px-2 py-0.5 text-[10px] font-mono text-slate-300">
                  ${this.escHtml(p)}
                  <button class="btn-remove-pattern text-slate-500 hover:text-red-400 leading-none" data-category="${cat}" data-index="${i}">×</button>
                </span>
              `).join("")}
            </div>`
          : ""}
        <div class="flex items-center gap-1">
          <input class="pattern-input flex-1 rounded border border-slate-700 bg-slate-900 px-2 py-0.5 text-[10px] font-mono text-slate-200 outline-none focus:border-cyan-500"
            type="text" placeholder="${cat === "routing" ? "service-a -> service-b" : "db/prod/*"}"
            data-category="${cat}" />
          <button class="btn-add-pattern rounded border border-slate-700 bg-slate-800 px-2 py-0.5 text-[10px] text-slate-400 hover:text-slate-200" data-category="${cat}">+</button>
        </div>
      </div>
    `;
  }

  private escHtml(s: string): string {
    return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
  }

  private bindEvents(): void {
    this.root.getElementById("btn-save-scopes")?.addEventListener("click", () => {
      void this.save();
    });

    this.root.querySelectorAll<HTMLButtonElement>(".btn-add-pattern").forEach((btn) => {
      btn.addEventListener("click", () => {
        const cat = btn.dataset.category as ScopeCategory;
        const input = this.root.querySelector<HTMLInputElement>(`.pattern-input[data-category="${cat}"]`);
        if (!input) return;
        this.addPattern(cat, input.value.trim());
      });
    });

    this.root.querySelectorAll<HTMLButtonElement>(".btn-remove-pattern").forEach((btn) => {
      btn.addEventListener("click", () => {
        const cat = btn.dataset.category as ScopeCategory;
        const idx = parseInt(btn.dataset.index ?? "0", 10);
        this.removePattern(cat, idx);
      });
    });

    this.root.querySelectorAll<HTMLButtonElement>(".btn-remove-category").forEach((btn) => {
      btn.addEventListener("click", () => {
        const cat = btn.dataset.category as ScopeCategory;
        const updated = { ...this.scopes };
        delete updated[cat];
        this.scopes = updated;
        this.allowAll = false;
        this.dirty = true;
        this.validationError = "";
        this.render();
        this.bindEvents();
      });
    });

    this.root.querySelectorAll<HTMLInputElement>(".pattern-input").forEach((input) => {
      input.addEventListener("keydown", (e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          const cat = input.dataset.category as ScopeCategory;
          this.addPattern(cat, input.value.trim());
        }
      });
    });
  }

  private addPattern(cat: ScopeCategory, pattern: string): void {
    if (!pattern) return;

    const isRouting = cat === "routing";
    const valid = isRouting ? isValidRoutingTuple(pattern) : isValidGlob(pattern);
    if (!valid) {
      this.validationError = isRouting
        ? `Invalid routing tuple — must contain ' -> ' (e.g. "service-a -> service-b")`
        : `Invalid glob pattern: '${pattern}'`;
      this.render();
      this.bindEvents();
      return;
    }

    this.validationError = "";
    const current = this.scopes[cat] ?? [];
    this.scopes = { ...this.scopes, [cat]: [...current, pattern] };
    this.allowAll = false;
    this.dirty = true;
    this.render();
    this.bindEvents();
  }

  private removePattern(cat: ScopeCategory, idx: number): void {
    const current = this.scopes[cat] ?? [];
    const next = current.filter((_, i) => i !== idx);
    const updated = { ...this.scopes };
    if (next.length === 0) {
      delete updated[cat];
    } else {
      updated[cat] = next;
    }
    this.scopes = updated;
    this.allowAll = Object.keys(this.scopes).length === 0;
    this.dirty = true;
    this.validationError = "";
    this.render();
    this.bindEvents();
  }

  private async save(): Promise<void> {
    try {
      const payload: ScopesMap | "allow-all" = Object.keys(this.scopes).length === 0
        ? "allow-all"
        : this.scopes;
      await writeRouteScopes(this.routePath, payload);
      this.dirty = false;
      this.validationError = "";
      window.dispatchEvent(new CustomEvent("toast", {
        detail: { type: "success", message: `Scopes saved for ${this.routePath}` },
      }));
      await this.load();
    } catch (error) {
      const msg = error instanceof Error ? error.message : String(error);
      window.dispatchEvent(new CustomEvent("toast", { detail: { type: "error", message: msg } }));
    }
  }
}

customElements.define("tachyon-scopes-panel", TachyonScopesPanel);

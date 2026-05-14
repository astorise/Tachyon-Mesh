import stylesheetText from "../../style.css?inline";
import { trapFocus } from "../../utils/a11y";
import { resilientInvoke as invoke } from "../../utils/network";
import { t } from "../../utils/i18n";

export type BundleConflict = {
  name: string;
  bundledVersion: string;
  clusterVersion: string;
};

type BundleApplyOutcome = {
  success: boolean;
  message: string;
  configVersion: number;
  conflicts: BundleConflict[];
  requiresResolution: boolean;
};

type BundleDependency = {
  name: string;
  versionConstraint: string;
  source?: string | null;
};

type Resolution = "cluster" | "local";

const modalStylesheet = new CSSStyleSheet();
modalStylesheet.replaceSync(stylesheetText);

export class TachyonBundleConflictModal extends HTMLElement {
  private readonly root: ShadowRoot;
  private conflicts: BundleConflict[] = [];
  private resolutions = new Map<string, Resolution>();
  private busy = false;
  private removeFocusTrap: () => void = () => undefined;

  constructor() {
    super();
    this.root = this.attachShadow({ mode: "open" });
    this.root.adoptedStyleSheets = [modalStylesheet];
  }

  open(conflicts: BundleConflict[]): void {
    this.conflicts = conflicts.map((conflict) => ({ ...conflict }));
    this.resolutions.clear();
    this.busy = false;
    this.render();
  }

  connectedCallback(): void {
    this.render();
  }

  private render(): void {
    if (this.conflicts.length === 0) {
      this.removeFocusTrap();
      this.root.innerHTML = `<aside class="hidden"></aside>`;
      return;
    }
    const rows = this.conflicts
      .map((conflict) => {
        const resolution = this.resolutions.get(conflict.name);
        const clusterActive = resolution === "cluster" ? "ring-2 ring-cyan-300" : "";
        const localActive = resolution === "local" ? "ring-2 ring-amber-300" : "";
        return `
          <li class="rounded border border-slate-800 bg-slate-950/50 p-3 space-y-2" data-conflict="${this.escapeAttr(conflict.name)}">
            <div class="flex items-center justify-between text-xs">
              <span class="font-mono text-cyan-300">${this.escape(conflict.name)}</span>
              <span class="text-slate-400">${t("bundle.conflict.bundled")} <span class="font-mono text-slate-200">${this.escape(conflict.bundledVersion)}</span> · ${t("bundle.conflict.cluster")} <span class="font-mono text-slate-200">${this.escape(conflict.clusterVersion)}</span></span>
            </div>
            <div class="grid grid-cols-2 gap-2 text-[11px]">
              <button data-action="cluster" data-name="${this.escapeAttr(conflict.name)}" class="rounded border border-cyan-500/40 bg-cyan-500/10 px-2 py-1 text-cyan-200 hover:bg-cyan-500/20 ${clusterActive}">${t("bundle.conflict.use-cluster")}</button>
              <button data-action="local" data-name="${this.escapeAttr(conflict.name)}" class="rounded border border-amber-500/40 bg-amber-500/10 px-2 py-1 text-amber-200 hover:bg-amber-500/20 ${localActive}">${t("bundle.conflict.force-local")}</button>
            </div>
          </li>
        `;
      })
      .join("");
    const allResolved = this.conflicts.every((conflict) => this.resolutions.has(conflict.name));
    this.root.innerHTML = `
      <div class="fixed inset-0 z-[110] flex items-center justify-center bg-slate-950/85 backdrop-blur-sm" role="dialog" aria-modal="true" aria-labelledby="conflict-modal-title">
        <div class="w-[min(40rem,calc(100vw-2rem))] rounded-lg border border-amber-500/40 bg-slate-900 p-5 shadow-[0_0_28px_rgba(251,191,36,0.18)]" id="conflict-modal-panel">
          <header class="mb-3">
            <h3 id="conflict-modal-title" class="text-amber-300 text-base font-semibold">${t("bundle.conflict.title")}</h3>
            <p class="text-xs text-slate-400 mt-1">${t("bundle.conflict.intro")}</p>
          </header>
          <ul class="space-y-2 mb-4" aria-label="${t("bundle.conflict.list-label")}">${rows}</ul>
          <div class="flex justify-end gap-2 text-xs" role="group" aria-label="${t("bundle.conflict.actions-label")}">
            <button id="btn-cancel" class="rounded border border-slate-700 bg-slate-800 px-3 py-2 text-slate-300 hover:bg-slate-700">${t("bundle.conflict.cancel")}</button>
            <button id="btn-retry" ${!allResolved || this.busy ? "disabled" : ""} aria-disabled="${!allResolved || this.busy}" class="rounded border border-cyan-500/50 bg-cyan-500/15 px-3 py-2 text-cyan-200 hover:bg-cyan-500/25 disabled:opacity-50 disabled:cursor-not-allowed">${this.busy ? t("bundle.feedback.applying") : t("bundle.conflict.retry")}</button>
          </div>
        </div>
      </div>
    `;

    // Install the shared focus trap; Escape cancels the conflict dialog.
    this.removeFocusTrap();
    const dialog = this.root.querySelector<HTMLElement>('[role="dialog"]');
    if (dialog) {
      this.removeFocusTrap = trapFocus(dialog, () => {
        this.conflicts = [];
        this.resolutions.clear();
        this.root.innerHTML = "";
      });
    }

    this.root.querySelectorAll<HTMLButtonElement>("button[data-action]").forEach((button) => {
      button.addEventListener("click", () => {
        const action = button.dataset.action as Resolution;
        const name = button.dataset.name ?? "";
        if (!action || !name) return;
        this.resolutions.set(name, action);
        this.render();
      });
    });

    this.root.getElementById("btn-cancel")?.addEventListener("click", () => {
      this.conflicts = [];
      this.resolutions.clear();
      this.render();
    });

    this.root.getElementById("btn-retry")?.addEventListener("click", () => {
      void this.retry();
    });
  }

  private async retry(): Promise<void> {
    this.busy = true;
    this.render();
    const dependencies: BundleDependency[] = this.conflicts.map((conflict) => {
      const resolution = this.resolutions.get(conflict.name) ?? "cluster";
      if (resolution === "local") {
        return {
          name: conflict.name,
          versionConstraint: `=${conflict.bundledVersion}`,
          source: `./assets/${conflict.name}.wasm`,
        };
      }
      return {
        name: conflict.name,
        versionConstraint: `^${conflict.clusterVersion}`,
        source: null,
      };
    });
    try {
      const outcome = await invoke<BundleApplyOutcome>("bundle_and_apply_manifest", {
        dependencies,
      });
      if (outcome.requiresResolution && outcome.conflicts.length > 0) {
        this.open(outcome.conflicts);
        return;
      }
      window.dispatchEvent(
        new CustomEvent("app:notify", {
          detail: {
            type: "success",
            message: `${outcome.message} (config_version=${outcome.configVersion})`,
          },
        }),
      );
      this.conflicts = [];
      this.resolutions.clear();
      this.render();
    } catch (error) {
      this.busy = false;
      this.render();
      window.dispatchEvent(
        new CustomEvent("app:notify", {
          detail: {
            type: "error",
            message: error instanceof Error ? error.message : String(error),
          },
        }),
      );
    }
  }

  private escape(value: string): string {
    return value
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#039;");
  }

  private escapeAttr(value: string): string {
    return value.replace(/"/g, "&quot;");
  }
}

customElements.define("tachyon-bundle-conflict-modal", TachyonBundleConflictModal);

import { tachyonSharedStylesheet } from "../../styles/shared-sheets";
import { t } from "../../utils/i18n";

export class TachyonPolicyFormBadge extends HTMLElement {
  private readonly root: ShadowRoot;
  private readonly onLanguageChanged = () => this.render();

  constructor() {
    super();
    this.root = this.attachShadow({ mode: "open" });
    this.root.adoptedStyleSheets = [tachyonSharedStylesheet];
  }

  connectedCallback(): void {
    window.addEventListener("i18n:language-changed", this.onLanguageChanged);
    this.render();
  }

  disconnectedCallback(): void {
    window.removeEventListener("i18n:language-changed", this.onLanguageChanged);
  }

  private render(): void {
    const label = t("policy-form-badge.label");
    const tooltip = t("policy-form-badge.tooltip");
    this.root.innerHTML = `
      <span
        title="${tooltip}"
        class="inline-flex items-center rounded border border-slate-600 bg-slate-800/60 px-2 py-0.5 text-[10px] font-medium uppercase tracking-widest text-slate-400 cursor-help ml-2"
      >${label}</span>
    `;
  }
}

customElements.define("tachyon-policy-form-badge", TachyonPolicyFormBadge);

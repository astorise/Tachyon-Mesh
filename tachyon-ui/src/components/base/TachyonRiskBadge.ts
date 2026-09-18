/// Reusable risk badge component shown alongside concurrency / consistency
/// selectors. Renders a colored pill (green / amber / red) with a tooltip
/// explaining the concrete failure scenario for the current selection. The
/// `data-sim-scenario` attribute is forwarded so a future JS simulation script
/// can attach interactive demos without touching the component itself.

export type RiskLevel = "low" | "medium" | "high";

function escapeAttr(value: string): string {
  return value.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

const COLORS: Record<RiskLevel, { bg: string; text: string; label: string }> = {
  low: { bg: "bg-emerald-900/40", text: "text-emerald-300", label: "Low Risk" },
  medium: { bg: "bg-amber-900/40", text: "text-amber-300", label: "Medium Risk" },
  high: { bg: "bg-red-900/40", text: "text-red-300", label: "High Risk" },
};

export class TachyonRiskBadge extends HTMLElement {
  static get observedAttributes() {
    return ["level", "tooltip", "sim-scenario"];
  }

  attributeChangedCallback(): void {
    this.render();
  }

  connectedCallback(): void {
    this.render();
  }

  private render(): void {
    const level = (this.getAttribute("level") || "low") as RiskLevel;
    const tooltip = this.getAttribute("tooltip") || "";
    const sim = this.getAttribute("sim-scenario") || "";
    const colors = COLORS[level] || COLORS.low;
    const escTooltip = escapeAttr(tooltip);
    const escSim = escapeAttr(sim);
    // escapeAttr() covers every interpolated value below.
    // eslint-disable-next-line no-restricted-properties
    this.innerHTML = `
      <span
        class="inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-xs font-mono ${colors.bg} ${colors.text} cursor-help"
        title="${escTooltip}"
        data-sim-scenario="${escSim}"
      >
        ${level === "low" ? "●" : level === "medium" ? "▲" : "■"}
        ${colors.label}
      </span>
    `;
  }
}

if (!customElements.get("tachyon-risk-badge")) {
  customElements.define("tachyon-risk-badge", TachyonRiskBadge);
}

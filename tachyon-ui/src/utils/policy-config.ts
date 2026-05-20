import { resilientInvoke as invoke } from "./network";
import { t } from "./i18n";

export type ConfigSource = "active" | "staged" | "defaults";

export type PolicyConfig = {
  payload: Record<string, unknown>;
  source: ConfigSource;
};

/**
 * Load configuration for a policy-form panel.
 * Priority: active (sealed on cluster) > staged (overlay) > null (use defaults).
 */
export async function loadPolicyConfig(domain: string): Promise<PolicyConfig | null> {
  try {
    const active = await invoke<Record<string, unknown> | null>("get_active_config", { domain });
    if (active) return { payload: active, source: "active" };
  } catch { /* offline or older node */ }

  try {
    const staged = await invoke<Record<string, unknown> | null>("get_staged_config", { domain });
    if (staged) return { payload: staged, source: "staged" };
  } catch { /* overlay unreadable */ }

  return null;
}

/** Renders a small pill indicating where the displayed values come from. */
export function configSourceBadge(source: ConfigSource): string {
  const label = t(`policy.config-source.${source}`);
  const colors: Record<ConfigSource, string> = {
    active: "border-emerald-500/40 bg-emerald-500/10 text-emerald-300",
    staged: "border-amber-500/40 bg-amber-500/10 text-amber-300",
    defaults: "border-slate-700 bg-slate-800/60 text-slate-400",
  };
  return `<span class="inline-flex items-center rounded border px-2 py-0.5 text-[10px] font-mono ml-2 ${colors[source]}">${label}</span>`;
}

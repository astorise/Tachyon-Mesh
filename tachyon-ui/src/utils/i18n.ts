export type SupportedLanguage = "en" | "fr";

type Dictionary = Record<string, string>;

const storageKey = "tachyon:ui:language";

const dictionaries: Record<SupportedLanguage, Dictionary> = {
  en: {
    "nav.dashboard": "Dashboard",
    "nav.overview": "Overview",
    "nav.topology": "Mesh Topology",
    "nav.registry": "Asset Registry",
    "nav.routing": "Routing",
    "nav.resilience": "Resilience",
    "nav.ai": "AI Orchestration",
    "nav.hardware": "Hardware",
    "nav.identity-config": "Identity & Quotas",
    "nav.rbac": "RBAC",
    "nav.workloads": "Workloads",
    "nav.observability": "Observability",
    "nav.storage": "Storage",
    "nav.fleet": "Fleet",
    "nav.supply-chain": "Supply Chain",
    "shell.title": "Control Plane",
    "shell.subtitle": "Native Web Components shell",
    "shell.operator": "Operator",
    "shell.unknown": "unknown",
    "shell.version": "v1.0.0-webcomponents",
    "shell.help": "Help / Tour",
    "shell.language": "Language",
    "dashboard.wasm": "Wasm Engine",
    "dashboard.ready": "Ready",
    "dashboard.routing": "Routing",
    "dashboard.identity": "Identity",
    "overview.title.prefix": "Global",
    "overview.title.strong": "Overview",
    "overview.telemetry": "Mesh telemetry / boot sequence snapshot",
    "overview.loading": "Loading mesh telemetry...",
    "overview.online": "Mesh telemetry online",
    "overview.failed": "Telemetry fetch failed",
    "overview.failedToast": "Telemetry fetch failed",
    "overview.nodes.label": "Active Edge Nodes",
    "overview.nodes.detail": "Mesh members reporting healthy control-plane heartbeats",
    "overview.wasm.label": "Global Wasm Instances",
    "overview.wasm.detail": "Component workloads currently admitted across the fleet",
    "overview.gpu.label": "AI/GPU Utilization",
    "overview.gpu.detail": "Accelerator allocation across active AI routing targets",
    "overview.routing": "routing",
    "overview.routing.status": "sealed",
    "overview.identity": "identity",
    "overview.identity.status": "enforced",
    "overview.observability": "observability",
    "overview.observability.status": "ready",
    "tour.skip": "Skip",
    "tour.previous": "Previous",
    "tour.next": "Next",
    "tour.finish": "Finish",
    "tour.nav.title": "Navigation",
    "tour.nav.desc": "Switch between configuration areas, topology, registry, storage, fleet, and IAM views.",
    "tour.header.title": "Operator Controls",
    "tour.header.desc": "Change the UI language or relaunch this guided tour from the header.",
    "tour.auth.title": "Authenticated Session",
    "tour.auth.desc": "Login and enrollment finish with MFA before the control plane unlocks.",
    "tour.seal.title": "Seal & Apply",
    "tour.seal.desc": "Pending configuration changes must be sealed and applied with step-up MFA.",
    "tour.overview.title": "Live Overview",
    "tour.overview.desc": "Monitor mesh node targets, admitted Wasm work, and derived accelerator utilization.",
    "tour.observability.title": "Observability",
    "tour.observability.desc": "Use observability metrics to confirm the applied configuration is behaving correctly.",
    "tour.registry.title": "Asset Registry",
    "tour.registry.desc": "Open the registry route to publish and inspect runtime assets.",
  },
  fr: {
    "nav.dashboard": "Tableau de bord",
    "nav.overview": "Vue d'ensemble",
    "nav.topology": "Topologie du mesh",
    "nav.registry": "Registre des assets",
    "nav.routing": "Routage",
    "nav.resilience": "Résilience",
    "nav.ai": "Orchestration IA",
    "nav.hardware": "Matériel",
    "nav.identity-config": "Identité et quotas",
    "nav.rbac": "RBAC",
    "nav.workloads": "Workloads",
    "nav.observability": "Observabilité",
    "nav.storage": "Stockage",
    "nav.fleet": "Flotte",
    "nav.supply-chain": "Supply Chain",
    "shell.title": "Plan de contrôle",
    "shell.subtitle": "Shell natif Web Components",
    "shell.operator": "Opérateur",
    "shell.unknown": "inconnu",
    "shell.version": "v1.0.0-webcomponents",
    "shell.help": "Aide / Tour",
    "shell.language": "Langue",
    "dashboard.wasm": "Moteur Wasm",
    "dashboard.ready": "Prêt",
    "dashboard.routing": "Routage",
    "dashboard.identity": "Identité",
    "overview.title.prefix": "Vue",
    "overview.title.strong": "globale",
    "overview.telemetry": "Télémétrie du mesh / instantané de démarrage",
    "overview.loading": "Chargement de la télémétrie mesh...",
    "overview.online": "Télémétrie mesh en ligne",
    "overview.failed": "Échec de récupération de la télémétrie",
    "overview.failedToast": "Échec de récupération de la télémétrie",
    "overview.nodes.label": "Noeuds edge actifs",
    "overview.nodes.detail": "Membres du mesh avec heartbeats control-plane sains",
    "overview.wasm.label": "Instances Wasm globales",
    "overview.wasm.detail": "Workloads de composants admis sur l'ensemble de la flotte",
    "overview.gpu.label": "Utilisation IA/GPU",
    "overview.gpu.detail": "Allocation des accélérateurs sur les cibles IA actives",
    "overview.routing": "routage",
    "overview.routing.status": "scellé",
    "overview.identity": "identité",
    "overview.identity.status": "appliquée",
    "overview.observability": "observabilité",
    "overview.observability.status": "prête",
    "tour.skip": "Ignorer",
    "tour.previous": "Précédent",
    "tour.next": "Suivant",
    "tour.finish": "Terminer",
    "tour.nav.title": "Navigation",
    "tour.nav.desc": "Passez entre configuration, topologie, registre, stockage, flotte et vues IAM.",
    "tour.header.title": "Contrôles opérateur",
    "tour.header.desc": "Changez la langue de l'interface ou relancez ce tour guidé depuis l'en-tête.",
    "tour.auth.title": "Session authentifiee",
    "tour.auth.desc": "La connexion et l'enrolement se terminent par MFA avant d'ouvrir le plan de controle.",
    "tour.seal.title": "Sceller et appliquer",
    "tour.seal.desc": "Les changements de configuration en attente doivent etre scelles et appliques avec MFA renforcee.",
    "tour.overview.title": "Vue live",
    "tour.overview.desc": "Surveillez les cibles mesh, le travail Wasm admis et l'utilisation accélérateur dérivée.",
    "tour.observability.title": "Observabilite",
    "tour.observability.desc": "Utilisez les metriques d'observabilite pour confirmer que la configuration appliquee se comporte correctement.",
    "tour.registry.title": "Registre des assets",
    "tour.registry.desc": "Ouvrez la route registre pour publier et inspecter les assets runtime.",
  },
};

let currentLanguage = readInitialLanguage();

export function getLanguage(): SupportedLanguage {
  return currentLanguage;
}

export function setLanguage(language: string): SupportedLanguage {
  const nextLanguage: SupportedLanguage = language === "fr" ? "fr" : "en";
  if (currentLanguage === nextLanguage) {
    return currentLanguage;
  }
  currentLanguage = nextLanguage;
  localStorage.setItem(storageKey, currentLanguage);
  window.dispatchEvent(new CustomEvent("i18n:language-changed", { detail: { language: currentLanguage } }));
  return currentLanguage;
}

export function t(key: string): string {
  return dictionaries[currentLanguage][key] ?? dictionaries.en[key] ?? key;
}

export const i18n = {
  getLanguage,
  setLanguage,
  t,
};

declare global {
  interface Window {
    tachyonI18n?: typeof i18n;
  }
}

window.tachyonI18n = i18n;

function readInitialLanguage(): SupportedLanguage {
  try {
    return localStorage.getItem(storageKey) === "fr" ? "fr" : "en";
  } catch {
    return "en";
  }
}

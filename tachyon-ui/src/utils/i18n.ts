export type SupportedLanguage = "en" | "fr";

type Dictionary = Record<string, string>;

const storageKey = "tachyon:ui:language";

const dictionaries: Record<SupportedLanguage, Dictionary> = {
  en: {
    "nav.dashboard": "Dashboard",
    "nav.overview": "Overview",
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
    "tour.nav.desc": "Switch between configuration areas, supply chain, storage, fleet, and IAM views.",
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
    "tour.registry.desc": "Open the supply chain route to publish and inspect runtime assets.",
    "iam.title": "Tachyon AuthN",
    "iam.subtitle": "Zero-Trust Control Plane Access",
    "iam.placeholder.url": "Mesh Node URL (https://...)",
    "iam.placeholder.username": "Username",
    "iam.placeholder.password": "Password",
    "iam.placeholder.invite": "Paste invite token",
    "iam.placeholder.first-name": "First Name",
    "iam.placeholder.last-name": "Last Name",
    "iam.placeholder.username-auto": "Username (auto: firstname.lastname)",
    "iam.placeholder.password-min": "Password (8+ characters)",
    "iam.placeholder.password-confirm": "Confirm password",
    "iam.password.show": "Show",
    "iam.password.hide": "Hide",
    "iam.remember": "Remember login and password on this workstation",
    "iam.ca.none": "No custom CA loaded.",
    "iam.ca.save": "Save CA",
    "iam.ca.clear": "Clear CA",
    "iam.ca.saved": "Custom CA saved for this workstation.",
    "iam.ca.cleared": "Custom CA cleared.",
    "iam.ca.select-first": "Select a custom CA certificate first.",
    "iam.ca.loaded.prefix": "Custom CA loaded",
    "iam.ca.bytes": "bytes",
    "iam.authenticate": "Authenticate",
    "iam.register": "Register with Invite Token",
    "iam.back": "Back",
    "iam.mfa.prompt": "Enter the 6-digit code from your authenticator app.",
    "iam.mfa.verify": "Verify Code",
    "iam.signup.intro": "Validate a 24-hour invite token before creating the admin profile.",
    "iam.signup.back-to-login": "Back to Login",
    "iam.signup.validate-invite": "Validate Invite",
    "iam.signup.no-invite": "No invite loaded yet.",
    "iam.signup.stage-account": "Stage Account",
    "iam.signup.qr-title": "Authenticator QR",
    "iam.signup.qr-waiting": "Waiting for staged enrollment.",
    "iam.signup.session": "Session",
    "iam.signup.pending": "Pending",
    "iam.signup.manual-secret": "Manual Secret",
    "iam.signup.scan-when-ready": "Scan the QR code when available.",
    "iam.signup.finalize": "Finalize Enrollment",
    "iam.signup.roles": "roles",
    "iam.signup.scopes": "scopes",
    "iam.signup.none": "none",
    "iam.error.generic": "Authentication failed.",
    "iam.error.login-required": "Node URL, username and password are required.",
    "iam.error.no-session": "Login staged without an MFA session id.",
    "iam.error.no-staged-login": "No staged login session is active.",
    "iam.error.no-staged-signup": "No staged signup session is active.",
    "iam.error.totp-format": "Enter a 6-digit MFA code.",
    "iam.error.url-required": "Mesh Node URL is required for enrollment.",
    "iam.error.password-rules": "Password must be at least 8 characters and match confirmation.",
    "iam.error.qr": "Unable to render QR code.",
    "iam.credentials.stronghold-stored": "Credentials are stored by the native Stronghold secure enclave, not browser localStorage.",
    "iam.admin.title": "Generate Operator Invite",
    "iam.admin.subtitle": "Creates a single-use token and QR code for a new operator to provision their identity.",
    "iam.admin.generate": "Generate Token",
    "iam.admin.manual-token": "Manual Entry Token",
    "iam.admin.invite-generated": "Invite token generated.",
    "mfa.title": "Step-up Authentication",
    "mfa.body": "Please enter your TOTP code to authorize this sensitive modification.",
    "mfa.cancel": "Cancel",
    "mfa.verify": "Verify",
    "mfa.format-error": "Enter a 6-digit MFA code.",
    "mfa.cancelled": "Step-up authentication cancelled.",
    "observability.title": "Observability",
    "observability.subtitle": "Domain: config-observability / OTLP telemetry",
    "observability.metrics.title": "Runtime Metrics",
    "observability.metrics.error-rate": "Error rate",
    "observability.metrics.p50": "p50 latency (ms)",
    "observability.metrics.p99": "p99 latency (ms)",
    "observability.metrics.queue": "Queue depth",
    "observability.metrics.source": "Source",
    "observability.metrics.empty": "No metrics available — host is offline or unsealed.",
    "observability.logs.title": "Recent Logs",
    "observability.logs.empty": "No log lines reported.",
    "observability.shadow.title": "Shadow Divergences",
    "observability.shadow.empty": "No shadow divergences captured.",
    "observability.refresh": "Refresh telemetry",
    "observability.config.title": "OTLP Configuration",
    "observability.config.endpoint": "OTLP Endpoint URL",
    "observability.config.log-level": "Log Level",
    "observability.config.update": "Update Telemetry",
    "observability.feedback.empty": "Awaiting telemetry configuration.",
    "routing.preview.title": "Currently sealed routes",
    "routing.preview.empty": "No sealed routes — the integrity manifest is empty.",
    "routing.preview.column.name": "Name",
    "routing.preview.column.path": "Path",
    "routing.preview.column.targets": "Targets",
    "routing.preview.column.tee": "TEE",
    "storage.preview.title": "Workspace overlay resources",
    "storage.preview.empty": "No overlay resources — push assets or seal new ones to populate this list.",
    "storage.preview.column.name": "Name",
    "storage.preview.column.kind": "Type",
    "storage.preview.column.target": "Target",
    "storage.preview.column.pending": "Pending",
  },
  fr: {
    "nav.dashboard": "Tableau de bord",
    "nav.overview": "Vue d'ensemble",
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
    "tour.nav.desc": "Passez entre configuration, Supply Chain, stockage, flotte et vues IAM.",
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
    "tour.registry.desc": "Ouvrez la route Supply Chain pour publier et inspecter les assets runtime.",
    "iam.title": "Tachyon AuthN",
    "iam.subtitle": "Accès Zero-Trust au plan de contrôle",
    "iam.placeholder.url": "URL du noeud mesh (https://...)",
    "iam.placeholder.username": "Nom d'utilisateur",
    "iam.placeholder.password": "Mot de passe",
    "iam.placeholder.invite": "Coller le jeton d'invitation",
    "iam.placeholder.first-name": "Prénom",
    "iam.placeholder.last-name": "Nom",
    "iam.placeholder.username-auto": "Nom d'utilisateur (auto: prenom.nom)",
    "iam.placeholder.password-min": "Mot de passe (8+ caractères)",
    "iam.placeholder.password-confirm": "Confirmer le mot de passe",
    "iam.password.show": "Afficher",
    "iam.password.hide": "Masquer",
    "iam.remember": "Mémoriser identifiant et mot de passe sur ce poste",
    "iam.ca.none": "Aucune CA personnalisée chargée.",
    "iam.ca.save": "Enregistrer la CA",
    "iam.ca.clear": "Effacer la CA",
    "iam.ca.saved": "CA personnalisée enregistrée sur ce poste.",
    "iam.ca.cleared": "CA personnalisée effacée.",
    "iam.ca.select-first": "Sélectionnez d'abord un certificat CA personnalisé.",
    "iam.ca.loaded.prefix": "CA personnalisée chargée",
    "iam.ca.bytes": "octets",
    "iam.authenticate": "S'authentifier",
    "iam.register": "S'enrôler avec un jeton d'invitation",
    "iam.back": "Retour",
    "iam.mfa.prompt": "Saisissez le code à 6 chiffres de votre application d'authentification.",
    "iam.mfa.verify": "Vérifier le code",
    "iam.signup.intro": "Validez un jeton d'invitation (24 h) avant de créer le profil admin.",
    "iam.signup.back-to-login": "Retour à la connexion",
    "iam.signup.validate-invite": "Valider l'invitation",
    "iam.signup.no-invite": "Aucune invitation chargée pour l'instant.",
    "iam.signup.stage-account": "Préparer le compte",
    "iam.signup.qr-title": "QR de l'authentificateur",
    "iam.signup.qr-waiting": "En attente d'un enrôlement préparé.",
    "iam.signup.session": "Session",
    "iam.signup.pending": "En attente",
    "iam.signup.manual-secret": "Secret manuel",
    "iam.signup.scan-when-ready": "Scannez le QR dès qu'il est disponible.",
    "iam.signup.finalize": "Finaliser l'enrôlement",
    "iam.signup.roles": "rôles",
    "iam.signup.scopes": "scopes",
    "iam.signup.none": "aucun",
    "iam.error.generic": "Échec de l'authentification.",
    "iam.error.login-required": "URL du noeud, identifiant et mot de passe sont requis.",
    "iam.error.no-session": "Préparation de connexion sans identifiant de session MFA.",
    "iam.error.no-staged-login": "Aucune session de connexion préparée n'est active.",
    "iam.error.no-staged-signup": "Aucune session d'enrôlement préparée n'est active.",
    "iam.error.totp-format": "Saisissez un code MFA à 6 chiffres.",
    "iam.error.url-required": "L'URL du noeud mesh est requise pour l'enrôlement.",
    "iam.error.password-rules": "Le mot de passe doit faire au moins 8 caractères et correspondre à la confirmation.",
    "iam.error.qr": "Impossible de générer le QR.",
    "iam.credentials.stronghold-stored": "Les identifiants sont stockés par l'enclave sécurisée Stronghold, pas par le localStorage du navigateur.",
    "iam.admin.title": "Générer une invitation opérateur",
    "iam.admin.subtitle": "Crée un jeton à usage unique et un QR pour qu'un nouvel opérateur provisionne son identité.",
    "iam.admin.generate": "Générer le jeton",
    "iam.admin.manual-token": "Jeton manuel",
    "iam.admin.invite-generated": "Jeton d'invitation généré.",
    "mfa.title": "Authentification renforcée",
    "mfa.body": "Saisissez votre code TOTP pour autoriser cette modification sensible.",
    "mfa.cancel": "Annuler",
    "mfa.verify": "Vérifier",
    "mfa.format-error": "Saisissez un code MFA à 6 chiffres.",
    "mfa.cancelled": "Authentification renforcée annulée.",
    "observability.title": "Observabilité",
    "observability.subtitle": "Domaine: config-observability / télémétrie OTLP",
    "observability.metrics.title": "Métriques runtime",
    "observability.metrics.error-rate": "Taux d'erreur",
    "observability.metrics.p50": "Latence p50 (ms)",
    "observability.metrics.p99": "Latence p99 (ms)",
    "observability.metrics.queue": "File d'attente",
    "observability.metrics.source": "Source",
    "observability.metrics.empty": "Aucune métrique disponible — l'hôte est hors-ligne ou non scellé.",
    "observability.logs.title": "Logs récents",
    "observability.logs.empty": "Aucune ligne de log rapportée.",
    "observability.shadow.title": "Divergences shadow",
    "observability.shadow.empty": "Aucune divergence shadow capturée.",
    "observability.refresh": "Rafraîchir la télémétrie",
    "observability.config.title": "Configuration OTLP",
    "observability.config.endpoint": "URL endpoint OTLP",
    "observability.config.log-level": "Niveau de log",
    "observability.config.update": "Mettre à jour la télémétrie",
    "observability.feedback.empty": "En attente de configuration de la télémétrie.",
    "routing.preview.title": "Routes scellées actuelles",
    "routing.preview.empty": "Aucune route scellée — le manifeste d'intégrité est vide.",
    "routing.preview.column.name": "Nom",
    "routing.preview.column.path": "Chemin",
    "routing.preview.column.targets": "Cibles",
    "routing.preview.column.tee": "TEE",
    "storage.preview.title": "Ressources de l'overlay workspace",
    "storage.preview.empty": "Aucune ressource overlay — poussez des assets ou scellez-en pour peupler cette liste.",
    "storage.preview.column.name": "Nom",
    "storage.preview.column.kind": "Type",
    "storage.preview.column.target": "Cible",
    "storage.preview.column.pending": "En attente",
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

type ErrorPattern = {
  regex: RegExp;
  handler: (matches: RegExpMatchArray, language: SupportedLanguage) => string;
};

export class ErrorTranslator {
  private readonly patterns: ErrorPattern[] = [
    {
      regex: /AI WIT validation failed: kv_cache_size must be between (\d+) and (\d+) GB/,
      handler: (matches, language) =>
        language === "fr"
          ? `Validation IA echouee : le cache KV doit etre compris entre ${matches[1]} et ${matches[2]} Go.`
          : `AI validation failed: KV cache must be between ${matches[1]} and ${matches[2]} GB.`,
    },
    {
      regex: /metadata\.name and metadata\.environment are required/,
      handler: (_matches, language) =>
        language === "fr"
          ? "Erreur de routage : le nom et l'environnement sont obligatoires."
          : "Routing error: name and environment are mandatory.",
    },
    {
      regex: /otlp_endpoint must use http\(s\):\/\//,
      handler: (_matches, language) =>
        language === "fr"
          ? "Erreur observabilite : l'endpoint OTLP doit utiliser http:// ou https://."
          : "Observability error: OTLP endpoint must use http:// or https://.",
    },
    {
      regex: /Rate limit exceeded/,
      handler: (_matches, language) =>
        language === "fr"
          ? "Limite de debit atteinte : patientez avant de relancer cette operation."
          : "Rate limit exceeded: wait before retrying this operation.",
    },
  ];

  translate(rawError: string): string {
    for (const pattern of this.patterns) {
      const match = rawError.match(pattern.regex);
      if (match) {
        return pattern.handler(match, currentLanguage);
      }
    }
    return currentLanguage === "fr" ? `Erreur systeme : ${rawError}` : `System error: ${rawError}`;
  }
}

const backendErrorTranslator = new ErrorTranslator();

export function translateBackendError(rawError: string): string {
  return backendErrorTranslator.translate(rawError);
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

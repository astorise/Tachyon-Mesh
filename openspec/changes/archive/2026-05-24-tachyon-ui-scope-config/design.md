## Context

Tachyon-UI est une SPA web-component (Lit/TypeScript) sans framework lourd. Les panneaux de configuration existants (Volumes, Concurrency) suivent le pattern `TachyonConfigDashboard` : un Web Component qui rend un template HTML inline, lit les inputs DOM, et appelle `applyAndSeal()` via `utils/network.ts`. Le routing dashboard expose les routes comme des cartes cliquables. `TachyonObservabilityPanel` consomme déjà `GET /admin/metrics` pour les latences.

Le backend expose `GET /admin/manifest` (lecture du manifest courant) et `POST /admin/manifest` (application). Le champ `scopes:` est maintenant un objet de la forme `{ secrets: ["pattern"], kv: [...], ... }` ou la sentinelle `"allow-all"`. `GET /admin/metrics` retourne `scopeDenialTotal` (lifetime, toutes catégories) en plus des métriques latence/erreur existantes.

## Goals / Non-Goals

**Goals:**
- Panneau Scopes dans la vue détail d'une route : lire, éditer, sauvegarder les patterns glob par catégorie.
- Badge allow-all visible sur les cartes de route et dans le panneau.
- Widget denial counters dans TachyonObservabilityPanel.
- Validation inline des globs côté client (pas de regex, pattern simple : caractères interdits `{` non balancé, whitespace nu, etc.).

**Non-Goals:**
- Pas d'éditeur YAML raw — l'UI structure les patterns ; le YAML est généré côté controller.
- Pas de suggestion automatique de patterns (c'est le rôle de tachyon-mcp-scope-tools).
- Pas de nouveau endpoint core-host : tout passe par `GET /admin/manifest` et `POST /admin/manifest` existants.
- Pas de modification du WIT ni de core-host.

## Decisions

**D1 — Représentation interne des scopes comme `Map<Category, string[]>`**
Le composant maintient un `Map<string, string[]>` en mémoire. À la sérialisation, les catégories vides (tableau vide) sont omises du payload (= catégorie non liée au linker). La sentinelle `allow-all` est représentée par `allowAll: boolean` séparé pour éviter la confusion avec un tableau de patterns `["allow-all"]`.

Pourquoi pas YAML inline : la structure est trop répétitive pour un éditeur texte libre, et les erreurs de syntaxe sont difficiles à diagnostiquer. Le pattern chip-per-pattern est déjà utilisé pour les tags dans d'autres panneaux.

**D2 — Validation glob en JavaScript sans dépendance**
La validation inline utilise une fonction utilitaire légère (`isValidGlob(pattern: string): boolean`) qui rejette : patterns vides, accolades non balancées, barres obliques doubles consécutives hors `**`. Elle ne simule pas `globset` exactement mais couvre les erreurs de saisie fréquentes. L'erreur définitive reste côté server (`POST /admin/manifest` renvoie 400 si le pattern est invalide).

Pourquoi pas WASM globset : overhead trop élevé pour la validation UX, et les cas invalides sont rares en usage normal.

**D3 — Lecture du manifest courant via `GET /admin/manifest` à l'ouverture du panneau**
À l'ouverture de la route detail view, le Scopes panel fait un `GET /admin/manifest`, extrait le bloc `scopes:` de la route concernée, et initialise son état interne. Cela garantit que l'opérateur travaille sur l'état réel et non un état mis en cache par le frontend.

Pourquoi pas un store partagé : le manifest peut être modifié par d'autres sessions (admin CLI, MCP) ; une lecture fraîche à chaque ouverture évite les conflits silencieux.

**D4 — `POST /admin/manifest` en full-replace, non en patch**
L'UI lit le manifest complet, modifie le bloc `scopes:` de la route cible, et reposte le manifest entier. Ce comportement est cohérent avec le pattern utilisé par tous les panneaux existants (`applyAndSeal`). La logique de merge est dans `scopesController.ts` : lecture → merge → post.

**D5 — `scopeDenialTotal` depuis `GET /admin/metrics` pour le widget d'observabilité**
Le widget Scope Denials utilise `scopeDenialTotal` (champ scalaire du JSON de metrics) pour afficher un total. La décomposition par catégorie n'est pas exposée par l'admin JSON (elle est dans prometheus) ; le widget affiche donc uniquement le total + un lien vers les métriques prometheus pour le détail. Cela évite d'ajouter un endpoint prometheus dans l'UI.

Si une décomposition par catégorie est ultérieurement souhaitée, elle peut être ajoutée à `AdminRuntimeMetrics` sans changer cette spec.

## Risks / Trade-offs

- **[Risk] Race condition manifest** : si deux opérateurs éditent simultanément, le dernier POST écrase les changements de l'autre. → Mitigation : afficher un timestamp "Last saved" dans le panneau ; envisager un ETag/If-Match dans une version future.
- **[Risk] Validation client divergente** : la validation glob côté UI peut accepter un pattern que le backend rejette. → Mitigation : les erreurs 400 du POST sont affichées dans le feedback toast avec le message d'erreur du serveur.
- **[Risk] `GET /admin/manifest` peut être lent sur un nœud chargé** → Mitigation : skeleton loader pendant la lecture ; délai max de 5 secondes (timeout identique aux autres panneaux).

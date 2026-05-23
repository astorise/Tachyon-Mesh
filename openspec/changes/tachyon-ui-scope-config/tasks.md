## 1. Controller et utilitaires

- [x] 1.1 Créer `tachyon-ui/src/controllers/scopesController.ts` avec `readRouteScopes(routePath)` (GET /admin/manifest → extrait `scopes:`), `writeRouteScopes(routePath, scopes)` (merge + POST /admin/manifest), et `readScopeMetrics()` (GET /admin/metrics → champ `scopeDenialTotal`).
- [x] 1.2 Créer `tachyon-ui/src/utils/glob-validate.ts` avec `isValidGlob(pattern: string): boolean` et `isValidRoutingTuple(pattern: string): boolean` (vérifie présence de ` -> `).
- [x] 1.3 Ajouter le type `ScopesMap = Record<ScopeCategory, string[]>` et l'enum `ScopeCategory` dans `tachyon-ui/src/types/scopes.ts`.
- [x] 1.4 Unit-test `glob-validate.ts` : globs valides (`db/prod/*`, `https://api.example.com/**`), invalides (`{non-balancé`, pattern vide), tuples routing valides/invalides.

## 2. Composant TachyonScopesPanel

- [x] 2.1 Créer `tachyon-ui/src/components/routing/TachyonScopesPanel.ts` (Web Component étendant `TachyonConfigDashboard`) avec attribut `route-path`.
- [x] 2.2 Implémenter `connectedCallback` : appel `scopesController.readRouteScopes()` → initialise `_state: ScopesMap`, détecte allow-all, render.
- [x] 2.3 Implémenter le rendu HTML : section par catégorie avec label, liste de chips (pattern + bouton ×), bouton "Add pattern" ouvrant un input inline, badge "ALLOW ALL" ambre conditionnel.
- [x] 2.4 Implémenter la logique d'ajout de pattern : validation via `glob-validate.ts` → erreur inline ou ajout chip → activation du bouton "Save scopes".
- [x] 2.5 Implémenter la logique de suppression de pattern : clic × → retrait du chip → catégorie retourne à "Not granted" si vide.
- [x] 2.6 Implémenter la règle routing tuple : `isValidRoutingTuple` utilisé dans l'input de la catégorie `routing`, message d'erreur spécifique si manque ` -> `.
- [x] 2.7 Implémenter `save()` : appel `scopesController.writeRouteScopes()` → toast succès/erreur → refresh panel depuis manifest.
- [x] 2.8 Unit-test `TachyonScopesPanel` : allow-all badge présent quand `scopes` absent, badge absent quand scope explicite, routing tuple invalide bloque Save, pattern invalide bloque ajout.

## 3. Intégration dans la route detail view

- [x] 3.1 Dans la vue de détail d'une route (identifier le fichier concerné dans `tachyon-ui/src`), ajouter `<tachyon-scopes-panel route-path="...">` comme peer des panneaux Volumes et Concurrency.
- [x] 3.2 Implémenter l'état collapsed/expanded : pre-expanded quand allow-all détecté, collapsed avec chip-count quand scope explicite.
- [x] 3.3 Enregistrer `TachyonScopesPanel` dans `customElements.define` et l'importer dans le point d'entrée approprié.

## 4. Badge allow-all sur les cartes de route

- [x] 4.1 Dans le composant de carte de route (routing dashboard), lire le champ `scopes` du manifest lors du rendu des cartes.
- [x] 4.2 Afficher un pill ambre "allow-all" quand `scopes` est absent ou `"allow-all"`, et un indicateur vert "scoped" sinon.
- [x] 4.3 Implémenter le tooltip sur le badge : "This deployment grants all WIT imports. Click to configure scopes."
- [x] 4.4 Clic sur le badge → navigation vers la route detail view avec Scopes panel pre-expanded.

## 5. Widget Scope Denials dans TachyonObservabilityPanel

- [x] 5.1 Dans `TachyonObservabilityPanel.ts`, ajouter un bloc "Scope Denials" qui appelle `scopesController.readScopeMetrics()` à l'init et toutes les 30 secondes.
- [x] 5.2 Afficher `scopeDenialTotal` comme compteur principal avec un label "lifetime denials, all categories".
- [x] 5.3 Afficher l'état vide quand `scopeDenialTotal === 0` : "No scope denials recorded — scopes are working correctly."
- [x] 5.4 Ajouter un lien "Configure scopes" qui navigue vers le panneau Scopes de la route sélectionnée.
- [x] 5.5 Unit-test du widget : état vide, affichage du total, mise à jour au refresh.

## 6. Tests d'intégration et review

- [x] 6.1 Test d'intégration Vitest : `TachyonScopesPanel` avec mock de `scopesController` — scénario save happy path, scénario erreur API.
- [x] 6.2 Vérifier que `npm run build` passe sans warnings TypeScript.
- [x] 6.3 Vérifier que `npm run test` passe (tous les tests existants restent verts).
- [x] 6.4 Vérifier manuellement dans le navigateur (dev server) : panel Scopes visible, badge allow-all présent, save → toast, widget observabilité refresh.

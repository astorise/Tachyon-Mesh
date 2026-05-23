## 1. Enrichissement de tachyon_get_metrics

- [x] 1.1 Dans le handler `tachyon_get_metrics` de `tachyon-mcp/src/main.rs`, ajouter `"scope_denial_total": metrics.scope_denial_total` dans le JSON de réponse (champ déjà disponible dans `AdminRuntimeMetrics`).
- [x] 1.2 Mettre à jour la description de l'outil `tachyon_get_metrics` dans `tools/list` pour mentionner `scope_denial_total`.
- [x] 1.3 Ajouter un test dans `tachyon-mcp/tests/` vérifiant que la réponse de `tachyon_get_metrics` contient le champ `scope_denial_total`.

## 2. Outil tachyon_get_scope_denials

- [x] 2.1 Ajouter `"tachyon_get_scope_denials"` à la liste `tools/list` avec description, paramètre optionnel `route_path: string`, et note sur la décomposition per-catégorie via prometheus.
- [x] 2.2 Implémenter le handler dans `handle_tool_call` : appel `tachyon_client::get_metrics()` → extraire `scope_denial_total` → lire le manifest courant pour dériver `allow_all` de la route.
- [x] 2.3 Construire la réponse JSON : `{ route_path, scope_denial_total, allow_all, note }` pour une route spécifique, ou `{ routes: [...] }` sans filtre.
- [x] 2.4 Gérer l'erreur cluster unreachable : retourner JSON-RPC `-32001`.
- [x] 2.5 Ajouter `"tachyon_get_scope_denials"` au rate-limiter avec 30 req/min.
- [x] 2.6 Ajouter à `missing_required_args` : aucun argument requis (route_path est optionnel).
- [x] 2.7 Test unitaire : réponse avec route_path spécifié, réponse sans filtre, erreur unreachable.

## 3. Outil tachyon_set_route_scopes

- [x] 3.1 Ajouter `"tachyon_set_route_scopes"` à `tools/list` avec paramètres : `route_path: string` (requis), `scopes: object` (requis), `dry_run: bool` (optionnel, défaut false).
- [x] 3.2 Ajouter `"tachyon_set_route_scopes"` à `missing_required_args` avec `["route_path", "scopes"]`.
- [x] 3.3 Implémenter le handler : (1) `GET /admin/manifest` → trouver la route cible → retourner `-32602` si introuvable, (2) merger le bloc `scopes:` dans la route, (3) si `dry_run` → retourner `manifest_preview`, sinon `POST /admin/manifest`.
- [x] 3.4 Construire la réponse success : `{ success: true, route_path, scopes_applied, dry_run }`.
- [x] 3.5 Ajouter au rate-limiter : 1 req/min (même bucket que `tachyon_apply_manifest`).
- [x] 3.6 Test : happy path, dry_run preview, route introuvable → `-32602`, rate limit → `-32002`.

## 4. Outil tachyon_suggest_scopes

- [x] 4.1 Ajouter `"tachyon_suggest_scopes"` à `tools/list` avec paramètre requis `route_path: string`.
- [x] 4.2 Ajouter à `missing_required_args` : `["route_path"]`.
- [x] 4.3 Implémenter le handler : (1) `GET /admin/manifest` → lire `scopes:` de la route, (2) `GET /admin/metrics` → lire `scope_denial_total`, (3) construire la suggestion.
- [x] 4.4 Logique de suggestion : si allow-all + denials > 0 → suggérer `{ "<highest_denial_category>": ["**"] }` avec commentaire conservateur ; si allow-all + denials == 0 → retourner `suggested_scopes: null` avec rationale.
- [x] 4.5 Construire la réponse : `{ route_path, current_state, scope_denial_total, suggested_scopes_yaml, rationale, conservative_suggestion, apply_with: "tachyon_set_route_scopes" }`.
- [x] 4.6 Ajouter au rate-limiter : 30 req/min.
- [x] 4.7 Test : route allow-all avec denials → suggestion non-null, route allow-all sans denials → `suggested_scopes: null`, route déjà scopée → `current_state: "explicitly-scoped"`.

## 5. Validation et tests d'intégration

- [x] 5.1 Vérifier que `cargo check -p tachyon-mcp` passe sans warnings.
- [x] 5.2 Vérifier que `cargo test -p tachyon-mcp` passe (tests existants + nouveaux).
- [ ] 5.3 Test d'intégration end-to-end (si tachyon-client mockable) : enchaîner `tachyon_get_scope_denials` → `tachyon_suggest_scopes` → `tachyon_set_route_scopes` dry_run → vérifier le flux complet.
- [x] 5.4 Vérifier que les 3 nouveaux outils apparaissent dans `tools/list` et que leurs descriptions mentionnent les champs clés (route_path, scopes, dry_run).

## Why

Le système de scoping FaaS (faas-wit-import-scoping) expose des métriques de refus et permet de configurer les patterns d'autorisation via le manifest. Mais aucun outil MCP ne permet à Claude de lire l'état des refus, de suggérer des patterns minimaux, ni de les appliquer — l'opérateur doit sortir du contexte conversationnel pour manipuler l'YAML manuellement. Cela bloque l'adoption de la phase 2 du plan de migration (tightening via assistant IA).

## What Changes

- **`tachyon_get_scope_denials`** : lit `GET /admin/metrics` et retourne un résumé structuré des refus de scope par catégorie pour une route donnée (ou toutes les routes).
- **`tachyon_set_route_scopes`** : applique un bloc `scopes:` à une route via `POST /admin/manifest`, avec dry-run optionnel et validation préalable du schema.
- **`tachyon_suggest_scopes`** : analyse les compteurs de refus actuels et produit un bloc `scopes:` suggéré en mode `allow-all` vers un scope minimal (phase 2 du plan de migration). Output en YAML prêt à coller.
- Rate-limiting conservateur pour les outils de mutation (1 req/min pour `tachyon_set_route_scopes`).

## Capabilities

### New Capabilities

- `tachyon-mcp-scope-tools`: Trois outils MCP pour lire les refus de scope, suggérer des patterns minimaux, et appliquer un bloc `scopes:` à une route.

### Modified Capabilities

- `mcp-advanced-tools`: `tachyon_get_metrics` est étendu avec le champ `scope_denial_total` déjà exposé par core-host — pas de changement de contrat, enrichissement du résultat retourné.

## Impact

- `tachyon-mcp/src/main.rs` : 3 nouveaux outils dans `tools/list` et `tools/call`
- `tachyon-mcp/tests/` : tests d'intégration pour les 3 outils
- Dépend de `GET /admin/metrics` (champ `scopeDenialTotal`) et `POST /admin/manifest` (champ `scopes:`) déjà disponibles dans core-host
- Pas de changement à WIT, core-host, ou tachyon-ui

## Context

`tachyon-mcp/src/main.rs` est un serveur JSON-RPC 2.0 en Rust (stdio) qui expose des outils MCP. Les outils existants lisent l'état du nœud via `tachyon_client` (crate interne) et mutent via `POST /admin/manifest`. Le rate-limiter global protège les outils mutants. `tachyon_get_metrics` retourne déjà un sous-ensemble des champs de `AdminRuntimeMetrics` ; le nouveau champ `scopeDenialTotal` sera ajouté au mapping existant.

`GET /admin/metrics` retourne maintenant `scopeDenialTotal` (u64, lifetime total). La décomposition par catégorie et par déploiement est dans prometheus ; le MCP ne parle pas directement à prometheus (pas d'endpoint prometheus dans tachyon-client). `tachyon_suggest_scopes` devra donc inférer les patterns depuis les arguments rejetés — information non disponible directement dans les métriques actuelles.

## Goals / Non-Goals

**Goals:**
- `tachyon_get_scope_denials` : lecture des métriques de refus aggregées.
- `tachyon_set_route_scopes` : mutation du manifest pour appliquer un bloc `scopes:`.
- `tachyon_suggest_scopes` : suggestion de patterns basée sur l'état courant (deny/allow-all).
- Enrichir le résultat de `tachyon_get_metrics` avec `scope_denial_total`.

**Non-Goals:**
- Pas d'accès direct prometheus depuis le MCP (pas d'endpoint disponible dans tachyon-client).
- `tachyon_suggest_scopes` ne génère pas de patterns depuis les arguments des requêtes rejetées (information non enregistrée) ; il suggère de partir sur `allow-all` pour les catégories sans données et fournit le total par catégorie si disponible.
- Pas de WebSocket/streaming pour le suivi en temps réel (hors scope MCP stdcio).

## Decisions

**D1 — `tachyon_get_scope_denials` agrège depuis `GET /admin/metrics`**
Le champ `scopeDenialTotal` du JSON admin est le seul point d'accès disponible sans prometheus. Le tool retourne ce total global + le flag `allow_all` dérivé du manifest courant (si `scopes: "allow-all"` ou absent). Une décomposition par catégorie n'est pas possible sans un endpoint dédié ou un accès prometheus — le tool le documente explicitement dans son output (`"note": "Per-category breakdown available via prometheus faas_scope_denials_total{deployment,category}"`).

Pourquoi ne pas ajouter un endpoint de détail dans core-host maintenant : cela dépasse le scope de ce change ; c'est une évolution naturelle pour `tachyon-mcp-scope-tools v2`.

**D2 — `tachyon_set_route_scopes` effectue read-merge-write**
Le tool : (1) `GET /admin/manifest` pour lire le manifest courant, (2) merge du bloc `scopes:` dans la route cible en mémoire, (3) `POST /admin/manifest` avec le manifest complet. Même pattern que `tachyon_apply_manifest` existant. Le mode `dry_run: true` saute l'étape 3 et retourne le payload qui aurait été posté.

**D3 — `tachyon_suggest_scopes` est informatif uniquement**
Le tool lit le manifest courant pour déterminer `current_state` (allow-all / partiellement scopé / entièrement scopé) et le total de refus depuis `GET /admin/metrics`. Il produit un YAML suggéré basé sur des heuristiques simples : si une catégorie a des refus, suggérer `["**"]` (accepter tout dans cette catégorie) comme point de départ sûr, avec un commentaire invitant à restreindre. Ce n'est pas une suggestion optimale mais un point de départ exploitable.

Pourquoi pas une analyse des arguments rejetés : les arguments des appels refusés ne sont pas loggués (privacy) et ne sont pas dans les métriques actuelles.

**D4 — Rate-limit de 1 req/min pour `tachyon_set_route_scopes`**
Même contrainte que `tachyon_apply_manifest` et `tachyon_seal_overlay`. Les outils de lecture (`tachyon_get_scope_denials`, `tachyon_suggest_scopes`) ont un rate-limit de 30 req/min identique à `tachyon_get_metrics`.

**D5 — Enrichissement de `tachyon_get_metrics` via mapping direct**
Le handler existant de `tachyon_get_metrics` lit `AdminRuntimeMetrics` et sérialise un sous-ensemble. On ajoute `"scope_denial_total": metrics.scope_denial_total` dans le JSON de réponse. Aucune breaking change : ajout additionnel.

## Risks / Trade-offs

- **[Risk] `tachyon_suggest_scopes` produit des suggestions trop larges (`**`)** → Mitigation : le YAML suggéré inclut des commentaires clairs indiquant que les patterns sont conservateurs et doivent être restreints manuellement. Le tool label son output `"conservative_suggestion": true`.
- **[Risk] `tachyon_set_route_scopes` écrase des modifications concurrentes** (race manifest) → Mitigation : le mode `dry_run` permet de prévisualiser avant d'appliquer ; documenter dans la description de l'outil.
- **[Risk] Absence de décomposition par catégorie dans `tachyon_get_scope_denials`** → Mitigation : le tool indique explicitement où trouver la décomposition (prometheus) ; une amélioration future pourra ajouter un endpoint `/admin/metrics/scopes` à core-host.

## Why

Les déploiements FaaS disposent maintenant d'un système de scoping par import (faas-wit-import-scoping), mais l'opérateur n'a aucune interface pour configurer les blocs `scopes:` ni pour visualiser les refus — il doit éditer l'YAML à la main et lire des métriques prometheus brutes. Cela crée un frein à l'adoption de la phase 2 du plan de migration (tightening).

## What Changes

- **Nouveau panneau Scopes** dans la vue de détail d'une route : éditeur structuré par catégorie (secrets, kv, vector, http, routing, outbox, storage, training, bridge, graph) avec champs de patterns glob, bouton allow-all avec badge WARNING, et validation inline.
- **Widget Scope Denials** dans `TachyonObservabilityPanel` : compteurs par catégorie pour le déploiement sélectionné, alimentés par `GET /admin/metrics` (`scopeDenialTotal`) et une fenêtre de taux de refus visuelle.
- **Badge allow-all** sur les cartes de route dans le dashboard : indicateur visuel rouge/orange quand un déploiement utilise encore `allow-all`.
- **Validation inline** : patterns glob invalides détectés côté client avant soumission ; règle `route-path → destination` pour la catégorie routing vérifiée avec un message d'erreur explicite.

## Capabilities

### New Capabilities

- `tachyon-ui-scope-editor`: Panneau d'édition des scopes par catégorie dans la vue détail d'une route, avec validation inline, soumission via `POST /admin/manifest`, et gestion de l'état allow-all.
- `tachyon-ui-scope-observability`: Widget de visualisation des refus de scope par catégorie dans le panneau d'observabilité existant.

### Modified Capabilities

- `tachyon-ui-route-config`: La vue de détail d'une route acquiert un panneau Scopes (s'ajoute aux panneaux Volumes et Concurrency déjà spécifiés).

## Impact

- `tachyon-ui/src/components/routing/` : nouveau composant `TachyonScopesPanel.ts`
- `tachyon-ui/src/components/domains/TachyonObservabilityPanel.ts` : widget de refus de scope
- `tachyon-ui/src/controllers/` : nouveau `scopesController.ts` pour la soumission manifest
- `tachyon-ui/src/utils/network.ts` : consommation de `GET /admin/metrics` (champ `scopeDenialTotal`) et de `GET /admin/schema/manifest` pour la validation inline
- Pas de changement au WIT, pas de changement à core-host

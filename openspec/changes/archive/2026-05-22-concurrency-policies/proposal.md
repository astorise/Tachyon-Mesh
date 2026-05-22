## Why

Les FaaS Tachyon sont par nature éphémères et exécutées en parallèle sur N nodes. Aujourd'hui, le runtime n'expose aucune politique de concurrence : toutes les invocations s'exécutent simultanément, tous les volumes acceptent `last_write_wins` sans option, et le scheduler de backup tourne **sur chaque node** (donc déclenche un backup par node à la même heure, gaspillant la bande passante et risquant des collisions de snapshots).

Les changes `s3-faas-volumes` et `s3-storage-backup` ont documenté ces limitations en phase 1 sans les résoudre. Cette feature les adresse via des modes déclaratifs explicites, à la manière des `accessModes` Kubernetes, en laissant le choix à l'utilisateur final selon son cas d'usage (batch vs interactif vs stateful).

## What Changes

- Nouveau champ `concurrency` sur `IntegrityRoute` avec mode `unrestricted | node-singleton | mesh-singleton | mesh-leader`
- Nouveau bloc `consistency: { read_mode, write_mode }` sur `IntegrityVolume`
- Nouveau bloc `coordination: { mode, write_isolation }` sur le champ `backup_schedule` (extension)
- Primitive de lock distribué dans `core-store` (`DistributedLock` table avec lease TTL)
- Primitive d'élection de leader basée sur le node registry
- Filtre d'admission dans le runtime qui rejette/met en attente selon `concurrency.mode`
- 1 outil MCP `recommend_concurrency_policy(pattern, requirements)` qui retourne une suggestion structurée
- UI : badges de risque par option, tooltips explicatifs, structure HTML préparée pour une future simulation JS

## Capabilities

### New Capabilities

- `concurrency-policies`: Politiques déclaratives de concurrence d'exécution FaaS (singleton intra-node / mesh-wide / leader), modes de cohérence des volumes (snapshot/live/etag/lock), et coordination des backups planifiés. Inclut les primitives de lock distribué et d'élection de leader nécessaires.

### Modified Capabilities

- `wasm-function-execution`: Le pipeline d'exécution évalue `concurrency.mode` de la route et bloque/met en attente les invocations qui dépasseraient la limite singleton.
- `volume-backup`: Le scheduler de backup honore `coordination.mode` (par défaut, un seul node élu exécute le backup au lieu de tous).
- `s3-faas-volumes`: Le commit S3 honore `consistency.write_mode` (LWW par défaut, optimistic_etag ou pessimistic_lock en opt-in).
- `mcp-server`: Nouvel outil `recommend_concurrency_policy`.
- `tachyon-ui-route-config`: Panneaux UI pour configurer les modes avec badges de risque + tooltips.

## Impact

- **core-host** : nouveau module `concurrency_admission.rs`, extension de `IntegrityRoute` et `IntegrityVolume`, extension du `CoreStore` avec une table `DistributedLock`, extension du backup scheduler avec coordination, intégration dans le pipeline d'exécution
- **tachyon-client** : 1 fonction `recommend_concurrency_policy`
- **tachyon-mcp** : 1 outil
- **tachyon-ui** : 1 panneau (intégré à la vue route detail), badges de risque réutilisables
- Dépendances : aucune nouvelle dépendance externe (utilise `redb` déjà présent pour le `DistributedLock`)
- Compatibilité : tous les champs nouveaux sont optionnels avec `Default::default()` = comportement actuel (`unrestricted`, `last_write_wins`, `per_node`), donc aucun manifest existant n'est cassé

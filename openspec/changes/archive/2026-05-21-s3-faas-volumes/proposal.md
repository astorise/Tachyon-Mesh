## Why

Tachyon délivre déjà `object_store` pour la persistance native (S3PersistenceBackend). Le coût marginal d'exposer S3 comme type de volume pour les FaaS utilisateurs est quasi nul — pas de nouvelle dépendance, pas de sidecar, pas de binary externe. Les fonctions WASM peuvent ainsi lire/écrire des objets S3 (datasets ML, fichiers médias, exports, sauvegardes) via l'API filesystem POSIX standard de WASI, sans changer leur code.

Sans cette feature, accéder à S3 depuis un guest requiert soit un client HTTP explicite dans le WASM (complexité), soit un volume host monté via un CSI S3 externe (infrastructure lourde).

## What Changes

- **`VolumeType::S3`** : nouveau variant dans l'enum, sérialisé `"s3"`. Le `host_path` devient une URL S3 (`s3://bucket/prefix`).
- **`preopen_route_volumes()` async** : pour les volumes S3, télécharge le préfixe S3 vers un répertoire temporaire avant l'exécution du guest, préouvre ce répertoire, puis upload après exécution si `!readonly`.
- **`S3VolumeManager`** : gestion du cycle de vie (temp dir création/nettoyage, pre/post hooks).
- **integrity.lock schema** : `volume.type = "s3"` accepté et validé. `host_path` en format `s3://bucket/prefix`.
- **Tachyon-UI — panneau S3 Volumes** : dans la vue de configuration de route, onglet "Volumes" affichant les volumes S3 (bucket, préfixe, accès, état sync) et permettant d'en ajouter/supprimer.
- **Tachyon-MCP** : 3 nouveaux outils : `list_s3_volumes`, `attach_s3_volume`, `detach_s3_volume`.

## Capabilities

### New Capabilities

- `s3-faas-volumes`: volumes S3 montés en WASI pour les fonctions utilisateur.
- `tachyon-ui-route-config`: panneau de configuration des routes dans Tachyon-UI (volumes, env vars, resources).

### Modified Capabilities

- `wasm-function-execution`: supporte `VolumeType::S3` via pre/post-exec sync.
- `mcp-server`: 3 outils de gestion des volumes S3 sur les routes.

## Impact

- **`core-host/src/host_core/domain_types.rs`**: `VolumeType::S3` + validation URL.
- **`core-host/src/host_core/component_hosts.rs`**: `preopen_route_volumes` devient async, branche S3.
- **`core-host/src/host_core/guest_runtime.rs`**: execution pipeline async pour volumes S3.
- **`core-host/src/host_core/integrity_config.rs`**: validation `host_path` S3 URL.
- **`tachyon-mcp/`**: `list_s3_volumes`, `attach_s3_volume`, `detach_s3_volume`.
- **`tachyon-ui/`**: composant `S3VolumesPanel`, intégré dans la vue route/function.
- Pas de changement WIT guest. Les WASM existants fonctionnent sans modification.

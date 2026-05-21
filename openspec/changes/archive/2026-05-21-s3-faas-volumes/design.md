## Context

`preopen_route_volumes()` est une fonction synchrone qui prépare le contexte WASI avant l'exécution d'un guest. Pour `VolumeType::Host` et `VolumeType::Ram`, il s'agit de créer des répertoires locaux et de les préouvrir. Pour `VolumeType::S3`, il faut effectuer un téléchargement réseau (async) avant l'exécution et un upload après. Ce changement rend le pipeline d'exécution conscient du cycle de vie S3.

Le `S3PersistenceBackend` (feature `s3-persistence`) est déjà dans `AppState`. Les volumes S3 FaaS utilisent le même client `object_store` avec une configuration potentiellement différente (autre bucket, autres credentials via Secret).

## Goals / Non-Goals

**Goals:**
- Guest WASM accède aux fichiers S3 via POSIX (read/write/readdir) sans modification de code.
- Support lecture seule (`readonly: true`) — pas d'upload post-exec.
- Support lecture-écriture — upload delta post-exec (fichiers modifiés uniquement).
- UI : visualisation et configuration des volumes S3 dans la vue route.
- MCP : outils pour AI agents pour attacher/détacher/inspecter les volumes S3.

**Non-Goals:**
- Sync temps-réel (inotify/fsevents) pendant l'exécution du guest.
- Multi-bucket avec credentials per-volume dans la phase 1 (utilise le backend S3 global).
- Versioning S3 / snapshots.

## Decisions

### D1: `host_path` = URL S3 `s3://bucket/prefix`

Réutilise le champ existant sans nouveau champ dans `IntegrityVolume`. Le parser extrait `bucket` et `prefix`. Compatible avec l'integrity.lock existant (champ string opaque).

### D2: Téléchargement complet avant exec, upload complet après (phase 1)

Le diff-based upload (upload uniquement les fichiers modifiés) est souhaitable mais complexe (mtime tracking dans un emptyDir). La phase 1 fait un upload de tous les fichiers après exec si `!readonly`. La latence supplémentaire est acceptable pour des fonctions de traitement de fichiers (batch, non-interactif).

Phase 2 : upload différentiel via comparaison ETag / mtime.

### D3: Répertoire temporaire par invocation

Chaque exécution de guest avec un volume S3 crée un répertoire temporaire dans `$TMPDIR/tachyon-s3-vol-<uuid>/`. Ce répertoire est nettoyé après l'upload. Cela garantit l'isolation entre invocations concurrentes du même guest.

### D4: `preopen_route_volumes` devient async

La fonction est appelée depuis les deux chemins d'exécution (legacy WASM et Component Model). Ces chemins sont déjà dans des contextes tokio (`spawn_blocking` pour le WASM, directement async pour les composants). On extrait le chargement S3 en amont dans un `prepare_s3_volumes()` async qui retourne les temp dirs à passer à la fonction synchrone.

### D5: Credentials S3 partagés avec la persistence en phase 1

Les volumes S3 FaaS utilisent les mêmes `TACHYON_S3_*` env vars que la persistence. Phase 2 ajoutera des credentials per-volume via Kubernetes Secrets.

### D6: UI — panneau "Volumes" dans la vue Route

La route detail view (si elle n'existe pas, elle est créée) reçoit un onglet "Volumes" listant les volumes configurés. Les volumes S3 affichent le bucket, le préfixe, le mode (RW/RO) et le dernier sync. Un bouton "Add S3 Volume" ouvre un modal de configuration. Le manifest (integrity.lock) est mis à jour via `POST /admin/manifest`.

### D7: MCP — outils de configuration programmatique

Les 3 outils opèrent sur le manifest live via l'admin API :
- `list_s3_volumes(route_path)` → GET /admin/manifest, filtre les volumes S3
- `attach_s3_volume(route_path, s3_url, guest_path, readonly)` → PATCH manifest, POST /admin/manifest
- `detach_s3_volume(route_path, guest_path)` → PATCH manifest, POST /admin/manifest

## Risks / Trade-offs

- **Latence** : le pré-téléchargement S3 ajoute latence à cold start (50-500ms selon la taille du préfixe). Acceptable pour des fonctions batch/async. Déconseillé pour des fonctions HTTP latency-sensitive.
- **Taille des volumes** : pas de limite configurée en phase 1. Ajouter `max_size_bytes` en phase 2.
- **Concurrence** : plusieurs invocations simultanées du même guest avec le même volume S3 en RW peuvent se piétiner (chacun voit l'état initial, le dernier à finir gagne). Acceptable en phase 1 ; phase 2 : advisory locking S3.

## Migration Plan

1. Ajouter `VolumeType::S3` (backward-compatible, pas de breaking change).
2. Implémenter `prepare_s3_volumes()` et `commit_s3_volumes()`.
3. Intégrer dans le pipeline guest_runtime.
4. Ajouter les outils MCP.
5. Ajouter le panneau UI Route Config avec S3 Volumes.

## Open Questions

- Faut-il limiter `VolumeType::S3` aux routes `role: user` uniquement ? → Oui, les routes système ont accès direct au filesystem host.
- Timeout pour le pré-téléchargement ? → Utiliser le `readinessProbe` timeout comme proxy ; configurable en phase 2.

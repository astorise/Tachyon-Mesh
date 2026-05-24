## Context

Tachyon dispose déjà d'un client S3 embarqué (`object_store` + `s3-persistence` feature) utilisé pour la persistence auth et les volumes S3 FaaS. Les volumes Host et RAM sont aujourd'hui purement locaux : leur contenu est perdu si le pod redémarre. La demande est d'ajouter un mécanisme de backup/restore explicite vers S3 pour ces volumes, sans remplacer le mode de fonctionnement actuel (les volumes restent locaux pendant l'exécution).

`build_s3_store(bucket)` dans `volumes.rs` est déjà une primitive partagée qui lit les `TACHYON_S3_*` env vars. Elle sera réutilisée telle quelle.

## Goals / Non-Goals

**Goals:**
- Backup on-demand d'un volume Host ou RAM d'une route vers `s3://bucket/backups/<route>/<guest_path>/<timestamp>/`
- Restore d'un snapshot vers le répertoire local du volume
- Liste des snapshots disponibles pour un volume
- Backup planifié via un champ `backup_schedule` (cron string) sur `IntegrityVolume`
- 3 outils MCP + 3 handlers admin + panneau UI dans la route detail view

**Non-Goals:**
- Backup incrémental (phase 1 : upload complet du répertoire)
- Chiffrement des backups (les volumes TDE sont déjà chiffrés côté guest)
- Backup des volumes S3 FaaS (ceux-ci sont déjà dans S3)
- Rétention automatique / pruning des anciens snapshots (phase 2)

## Decisions

### D1: Structure S3 des snapshots

Chemin : `s3://bucket/<backup_prefix>/<route_path_normalized>/<guest_path_normalized>/<unix_ts_ms>/`

- `backup_prefix` : variable d'env `TACHYON_S3_BACKUP_PREFIX` (défaut : `backups`), distincte du prefix de persistence auth pour éviter toute collision.
- `route_path_normalized` : `/api/my-fn` → `api_my-fn` (slashes → underscores, trim leading /).
- Timestamp en ms pour ordre lexicographique et unicité.

### D2: Nouveau module `volume_backup.rs` dans core-host

Plutôt qu'étendre `persistence.rs` (qui gère la persistence auth), un module dédié `volume_backup.rs` expose :
- `backup_volume(route_path, guest_path) -> Result<BackupSnapshot>`
- `restore_volume(route_path, guest_path, snapshot_id) -> Result<()>`
- `list_volume_backups(route_path, guest_path) -> Result<Vec<BackupSnapshot>>`

Ces fonctions réutilisent `build_s3_store()` depuis `volumes.rs` (déjà `pub(crate)`).

### D3: Nouveaux endpoints admin

```
POST /admin/volumes/backup   { route_path, guest_path }          → BackupSnapshot
POST /admin/volumes/restore  { route_path, guest_path, snapshot_id }  → ()
GET  /admin/volumes/backups  ?route_path=...&guest_path=...      → Vec<BackupSnapshot>
```

Ces endpoints requirent le PAT admin (même auth que `/admin/manifest`).

### D4: Champ `backup_schedule` dans `IntegrityVolume`

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub(crate) backup_schedule: Option<String>,  // cron expression, e.g. "0 3 * * *"
```

Un background worker dans `background_workers.rs` lit les routes scellées, identifie les volumes avec `backup_schedule`, et déclenche `backup_volume()` selon le planning. La lib `cron` (déjà présente dans le workspace ?) ou un parser cron simple sera utilisé.

### D5: Planification dans le background worker existant

Le pattern tokio `interval` est déjà utilisé dans `background_workers.rs`. On ajoute une tâche qui tourne toutes les minutes, évalue quels volumes sont dus pour backup (comparaison next_run vs now), et déclenche le backup de façon non-bloquante.

### D6: Pas de modification du pipeline d'exécution guest

Contrairement aux volumes S3 FaaS, les backups ne se produisent pas dans le chemin d'exécution des guests. Ils sont toujours triggered explicitement (admin API ou scheduler), jamais automatiquement après une invocation.

## Risks / Trade-offs

- **Cohérence** : un backup pendant une invocation en cours peut capturer un état intermédiaire. Mitigation : avertissement dans la doc ; phase 2 pourra ajouter un advisory lock.
- **Taille** : pas de limite en phase 1. Un volume RAM de plusieurs Go déclenchera un upload long. Mitigation : `max_backup_size_bytes` en phase 2.
- **`backup_schedule` invalide** : un cron mal formé doit être rejeté à la validation de l'integrity.lock. Le parser valide la syntaxe avant d'accepter le manifest.
- **Isolation backup_prefix** : si `TACHYON_S3_BACKUP_PREFIX` n'est pas configuré et que le prefix par défaut `backups` chevauche un objet S3 existant, le listing pourrait inclure des objets inattendus. Le prefix doit être documenté.

## Migration Plan

1. Ajouter le champ `backup_schedule` à `IntegrityVolume` (backward-compatible, `skip_serializing_if = "Option::is_none"`).
2. Implémenter `volume_backup.rs` et les 3 handlers admin.
3. Intégrer le scheduler dans `background_workers.rs`.
4. Ajouter les 3 fonctions dans `tachyon-client` et les 3 outils MCP.
5. Ajouter les 3 commandes Tauri et le panneau `TachyonVolumeBackupsPanel`.

## Open Questions

- Faut-il un endpoint `DELETE /admin/volumes/backups/{id}` dès la phase 1 ? → Non, rétention en phase 2.
- Le `backup_prefix` doit-il être un champ dans `IntegrityVolume` (per-volume) ou une variable d'env globale ? → Variable d'env globale en phase 1 pour la simplicité.

## 1. Schema & Validation (core-host)

- [x] 1.1 Ajouter le champ `backup_schedule: Option<String>` à `IntegrityVolume` dans `domain_types.rs`
- [x] 1.2 Ajouter la validation du cron dans `integrity_config.rs` — rejeter les expressions non parsables lors de la validation du manifest
- [x] 1.3 Ajouter `TACHYON_S3_BACKUP_PREFIX` env var (défaut `backups`) lue au démarrage

## 2. Module volume_backup (core-host)

- [x] 2.1 Créer `core-host/src/host_core/volume_backup.rs` avec `BackupSnapshot { snapshot_id, route_path, guest_path, timestamp_ms, object_count }` et les trois fonctions async : `backup_volume`, `restore_volume`, `list_volume_backups`
- [x] 2.2 Implémenter `backup_volume(route_path, guest_path, config)` — résoudre le host_path du volume, uploader vers `s3://<bucket>/<backup_prefix>/<route_normalized>/<guest_normalized>/<ts_ms>/`, retourner `BackupSnapshot`
- [x] 2.3 Implémenter `restore_volume(route_path, guest_path, snapshot_id, config)` — lister les objets sous le prefix snapshot, les télécharger dans le host_path local
- [x] 2.4 Implémenter `list_volume_backups(route_path, guest_path, config)` — lister les prefixes de niveau timestamp sous le prefix route/guest, retourner `Vec<BackupSnapshot>` trié desc
- [x] 2.5 Déclarer `mod volume_backup;` dans `host_core.rs`

## 3. Endpoints admin HTTP (core-host)

- [x] 3.1 Ajouter `POST /admin/volumes/backup` dans `app_runtime.rs` — requiert PAT admin, appelle `backup_volume`, retourne le `BackupSnapshot` en JSON
- [x] 3.2 Ajouter `POST /admin/volumes/restore` dans `app_runtime.rs` — requiert PAT admin, appelle `restore_volume`, retourne HTTP 204
- [x] 3.3 Ajouter `GET /admin/volumes/backups` dans `app_runtime.rs` — requiert PAT admin, appelle `list_volume_backups`, retourne `Vec<BackupSnapshot>` en JSON

## 4. Scheduler de backup planifié (core-host)

- [x] 4.1 Ajouter une tâche background dans `background_workers.rs` qui tourne toutes les 60 secondes, évalue les volumes avec `backup_schedule` du manifest scellé, et déclenche `backup_volume` pour les volumes dont le cron est échu
- [x] 4.2 Persister le `last_backup_unix` en mémoire par volume (HashMap dans l'AppState ou struct dédiée) pour éviter les doublons en cas de tick rapide

## 5. tachyon-client

- [x] 5.1 Ajouter le type `BackupSnapshot` (Serialize/Deserialize) dans `lib.rs`
- [x] 5.2 Implémenter `backup_volume(route_path, guest_path)` → `POST /admin/volumes/backup`
- [x] 5.3 Implémenter `restore_volume(route_path, guest_path, snapshot_id)` → `POST /admin/volumes/restore`
- [x] 5.4 Implémenter `list_volume_backups(route_path, guest_path)` → `GET /admin/volumes/backups`

## 6. MCP tools (tachyon-mcp)

- [x] 6.1 Ajouter `backup_volume`, `restore_volume`, `list_volume_backups` dans `missing_required_args`
- [x] 6.2 Ajouter les rate limits pour les trois outils
- [x] 6.3 Ajouter les définitions des trois outils dans `tools/list`
- [x] 6.4 Ajouter les trois arms dans `handle_tool_dispatch`

## 7. UI — Panneau Backups (tachyon-ui)

- [x] 7.1 Ajouter trois commandes Tauri dans `main.rs` : `backup_volume`, `restore_volume`, `list_volume_backups`
- [x] 7.2 Créer `TachyonVolumeBackupsPanel.ts` — liste les snapshots, bouton "Backup now", bouton "Restore" par snapshot, empty state
- [x] 7.3 Importer et intégrer `TachyonVolumeBackupsPanel` dans la vue détail route (à côté du `TachyonVolumesPanel` S3 existant)

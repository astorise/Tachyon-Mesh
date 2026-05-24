## Why

Tachyon stocke des données persistantes sur des volumes locaux (RAM, Host) qui sont liés au cycle de vie du pod. Si un pod redémarre ou est évincé sans S3 persistence configurée, les données sont perdues. Cette feature ajoute un mécanisme de backup/restore à la demande et planifié vers S3 pour les volumes de stockage utilisateur, indépendamment du client S3 embarqué utilisé pour la persistence auth.

## What Changes

- Nouvelle commande admin `POST /admin/volumes/backup` — déclenche un snapshot des volumes Host/RAM d'une route vers S3
- Nouvelle commande admin `POST /admin/volumes/restore` — restaure un snapshot S3 vers un volume local
- Nouvelle commande admin `GET /admin/volumes/backups` — liste les snapshots disponibles pour une route/volume
- Backup planifié configurable via integrity.lock (`backup_schedule` sur un volume)
- 3 outils MCP : `backup_volume`, `restore_volume`, `list_volume_backups`
- UI : panneau "Backups" dans la vue route (à côté du panneau Volumes S3 existant)

## Capabilities

### New Capabilities

- `volume-backup`: Backup et restore de volumes FaaS (Host, RAM) vers/depuis S3 via l'API admin, avec support de planification dans integrity.lock et UI de gestion.

### Modified Capabilities

- `s3-persistence`: L'infrastructure S3 existante (`S3PersistenceBackend`, `build_s3_store`) est réutilisée et exposée comme primitive partagée pour le backup de volumes.
- `mcp-server`: Ajout de 3 outils de gestion des backups de volumes.
- `tachyon-ui-route-config`: Ajout d'un panneau Backups dans la vue détail d'une route.

## Impact

- **core-host** : nouveau module `volume_backup.rs`, nouveaux handlers HTTP admin, extension de `IntegrityVolume` avec champ `backup_schedule` optionnel
- **tachyon-client** : 3 nouvelles fonctions async (`backup_volume`, `restore_volume`, `list_volume_backups`)
- **tachyon-mcp** : 3 nouveaux outils
- **tachyon-ui** : nouveau composant `TachyonVolumeBackupsPanel`, 3 nouvelles commandes Tauri
- Dépendance : `s3-persistence` feature déjà présente dans core-host, aucune nouvelle dépendance

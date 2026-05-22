## 1. Schema (core-host)

- [ ] 1.1 Ajouter `ConcurrencyPolicy`, `ConcurrencyMode`, `ConflictPolicy` enums dans `domain_types.rs` + champ `concurrency` sur `IntegrityRoute` (Default = Unrestricted)
- [ ] 1.2 Ajouter `VolumeConsistency`, `ReadMode`, `WriteMode` enums + champ `consistency` sur `IntegrityVolume` (Default = Snapshot + LastWriteWins)
- [ ] 1.3 Étendre `backup_schedule` : accepter soit une string cron (compat) soit un objet `BackupSchedule { cron, coordination, write_isolation }` via `serde(untagged)`
- [ ] 1.4 Validation : rejeter `consistency.write_mode = "pessimistic_lock"` si `concurrency.mode = "unrestricted"` (incompatible)
- [ ] 1.5 Validation : rejeter `coordination.write_isolation = "copy_on_write"` si le filesystem n'est pas détecté comme cow-capable (warn only en phase 1)

## 2. Primitive DistributedLock (core-host)

- [ ] 2.1 Ajouter `CoreStoreBucket::DistributedLocks` dans `store/mod.rs` + table redb
- [ ] 2.2 Créer `core-host/src/host_core/distributed_lock.rs` avec API `acquire(key, ttl) -> Result<LockGuard>`, `LockGuard::heartbeat()`, `Drop = release`
- [ ] 2.3 Publier les acquisitions dans `ConfigUpdateOutbox` pour la propagation inter-node
- [ ] 2.4 Consommer les events `ConfigUpdateOutbox` de type `lock-acquired` dans le subscriber existant pour invalider les acquisitions locales si un autre node a déjà le lock
- [ ] 2.5 Tests unitaires : acquisition concurrente locale, expiration TTL, heartbeat refresh

## 3. Primitive LeaderElection (core-host)

- [ ] 3.1 Créer `core-host/src/host_core/leader_election.rs` avec `am_i_leader(resource_key: &str) -> bool` basé sur `hash(key) % nodes.len()` et la liste des nodes actifs du `IntegrityConfig.registry.active_systems`
- [ ] 3.2 Tests : 1 node → toujours leader, 2 nodes → un seul leader par key, déterministe entre runs

## 4. Admission filter (core-host)

- [ ] 4.1 Créer `core-host/src/host_core/concurrency_admission.rs` avec `check(state, route) -> Result<AdmissionGuard, AdmissionError>`
- [ ] 4.2 Implémenter les 4 branches : Unrestricted, NodeSingleton (Mutex local), MeshSingleton (DistributedLock), MeshLeader (am_i_leader + 503)
- [ ] 4.3 Implémenter `on_conflict` : Queue (notify+wait), Reject (HTTP 409), Drop (silent discard avec 204)
- [ ] 4.4 Background task : heartbeat refresh pour les guards long-lived (toutes les ttl/2)
- [ ] 4.5 Intégrer `check()` dans `execute_guest` au tout début, avant `resolve_guest_module_path`

## 5. S3 volume consistency (core-host)

- [ ] 5.1 Étendre `S3VolumePrep` avec `write_mode: WriteMode` et `etag: Option<String>` (capturé au download initial)
- [ ] 5.2 Implémenter `commit_s3_volumes_optimistic_etag` : conditional PUT avec `If-Match`, retourne erreur sur 412
- [ ] 5.3 Implémenter `commit_s3_volumes_pessimistic_lock` : wrap prepare+execute+commit dans un `DistributedLock`
- [ ] 5.4 Dispatcher dans `commit_s3_volumes` selon le `write_mode`

## 6. Backup coordination (core-host)

- [ ] 6.1 Étendre `spawn_volume_backup_scheduler` : évaluer `backup_schedule.coordination` avant de déclencher le backup
- [ ] 6.2 Pour `MeshLeader`, vérifier `am_i_leader(route_path + guest_path)`
- [ ] 6.3 Pour `ManualOnly`, skip silencieusement
- [ ] 6.4 Pour `write_isolation: Drain`, intégrer avec admission filter pour bloquer les nouvelles invocations + attendre les actives
- [ ] 6.5 Pour `write_isolation: CopyOnWrite`, détecter btrfs/zfs et utiliser `cp --reflink=auto` (Linux uniquement), fallback warn

## 7. tachyon-client

- [ ] 7.1 Ajouter le type `ConcurrencyRecommendation` (Serialize/Deserialize)
- [ ] 7.2 Implémenter `recommend_concurrency_policy(pattern, requirements) -> Result<ConcurrencyRecommendation>` — table de décision inline, pas de HTTP call

## 8. MCP tool (tachyon-mcp)

- [ ] 8.1 Ajouter `recommend_concurrency_policy` à `missing_required_args` (required: `pattern`)
- [ ] 8.2 Ajouter le rate limit (100/min, c'est purement local sans I/O)
- [ ] 8.3 Ajouter la définition de l'outil dans `tools/list`
- [ ] 8.4 Ajouter l'arm dans `handle_tool_dispatch`

## 9. UI — Concurrency Policy panel (tachyon-ui)

- [ ] 9.1 Créer `TachyonRiskBadge.ts` (composant réutilisable Low/Medium/High avec tooltip slot)
- [ ] 9.2 Créer `TachyonConcurrencyPolicyPanel.ts` : selects pour les 3 dimensions, badge global + badges par volume
- [ ] 9.3 Implémenter la table de risque inline (TypeScript) — même logique que la recommendation MCP
- [ ] 9.4 Cacher dynamiquement les combinaisons incompatibles, surface warning + bouton "fix"
- [ ] 9.5 Ajouter `data-sim-scenario` sur chaque option pour le futur hook JS de simulation
- [ ] 9.6 Intégrer le panneau dans la route detail view (à côté de Volumes et Backups)
- [ ] 9.7 Ajouter les traductions i18n FR/EN pour les tooltips et libellés

## Context

Trois mécanismes de concurrence sont implicites aujourd'hui et causent des problèmes silencieux :

1. **Exécution FaaS** — `concurrency_limits` existe déjà par route mais ne couvre que le nombre max d'invocations *par node*. Aucune notion de singleton mesh-wide ou per-node strict.
2. **Écritures volume** — pour les volumes S3 (change `s3-faas-volumes`), chaque invocation télécharge l'état initial puis upload tout à la fin → last-write-wins silencieux entre invocations concurrentes (même node ou nodes différents).
3. **Backup planifié** — le scheduler tourne sur chaque node (change `s3-storage-backup`) → N backups simultanés à chaque tick cron.

Le `CoreStore` (redb embarqué) offre déjà les primitives nécessaires pour un lock distribué : transactions ACID, table-per-bucket, TTL via timestamps. Le node registry maintient la liste des nodes mesh actifs (`active_systems`), ce qui permet une élection de leader basée sur une stratégie déterministe (hash + node_id ou rotation par tick).

## Goals / Non-Goals

**Goals:**
- Modes déclaratifs explicites sur 3 dimensions (exécution, volume, backup) — opt-in, défauts compatibles
- Primitive `DistributedLock` ré-utilisable (acquérir/relâcher/heartbeat avec lease TTL)
- Primitive `LeaderElection` ré-utilisable (déterministe ou élection active)
- UI avec badges de risque (vert/orange/rouge) + tooltips de scénarios d'échec
- Outil MCP `recommend_concurrency_policy` qui retourne une suggestion structurée selon un pattern déclaré
- Structure HTML/CSS préparée pour une future simulation JS interactive

**Non-Goals:**
- Simulation JS interactive (préparée mais non implémentée — viendra dans un change UI dédié)
- Garanties strictes anti-byzantine (le lock distribué est "best-effort" basé sur clock sync + TTL, suffisant pour les opérations FaaS, pas pour de la transaction financière)
- Migration automatique de manifestes existants (compatible par défaut, l'utilisateur opt-in explicitement)
- Lock à granularité fine (clé arbitraire) en phase 1 — seulement par route et par volume

## Decisions

### D1: Lock distribué via CoreStore avec lease TTL

Nouvelle bucket `DistributedLocks` dans `CoreStore`. Chaque lock est une row `key → {node_id, acquired_ms, lease_ttl_ms, refresh_count}`. Les writes sont sérialisés par redb (un seul writer à la fois), ce qui donne déjà une sérialisation locale au node.

**Pour la cohérence inter-node** : chaque pod core-host écrit son lock dans son propre redb local + publie l'événement dans un outbox `ConfigUpdateOutbox` (déjà présent) consommé par les autres nodes via la sync existante. Un node ne peut acquérir un lock que si son `CoreStore` ne voit pas de lock actif (lease non expiré).

Cette approche est "best-effort" — fenêtre de course possible si la sync inter-node a un lag > TTL/2. **Alternatives considérées** :
- **Raft natif** : trop lourd pour phase 1, augmente la complexité opérationnelle
- **Lock S3 (objet `.lock`)** : nécessite S3 disponible, latence élevée. Garder en option future
- **etcd/Consul externe** : ajout de dépendance, contre la philosophie "embedded core" de Tachyon

Le choix : **redb + outbox sync, suffisant pour le cas FaaS éphémère**. Si un singleton mesh-wide est critique, l'utilisateur configurera un TTL court (5s) + un node de référence pour minimiser la fenêtre.

### D2: Élection de leader déterministe par défaut

Pour `mesh-leader` et `coordination.mode: mesh_leader`, on n'utilise PAS d'élection active (vote). On utilise une fonction déterministe :

```
leader(resource_key) = active_nodes[hash(resource_key) % len(active_nodes)]
```

Les nodes calculent localement qui est le leader en lisant leur copie du node registry. Pas de communication. Si la liste des nodes diverge entre nodes (fenêtre de propagation), il peut y avoir double exécution courte. Acceptable pour les backups, le scheduler etc.

**Alternative** : élection via lock distribué (D1) → un seul node "gagne". Plus correct mais plus coûteux. Reportée à phase 2, on l'ajoutera comme `coordination.mode: mesh_leader_strict`.

### D3: Schema — modes opt-in, défauts compatibles

Sur `IntegrityRoute` :
```rust
#[serde(default, skip_serializing_if = "is_default")]
pub(crate) concurrency: ConcurrencyPolicy,

pub(crate) struct ConcurrencyPolicy {
    pub(crate) mode: ConcurrencyMode,           // default: Unrestricted
    pub(crate) lock_ttl_ms: Option<u64>,        // default: 30_000
    pub(crate) on_conflict: ConflictPolicy,     // default: Queue
}

pub(crate) enum ConcurrencyMode { Unrestricted, NodeSingleton, MeshSingleton, MeshLeader }
pub(crate) enum ConflictPolicy { Queue, Reject, Drop }
```

Sur `IntegrityVolume` :
```rust
#[serde(default, skip_serializing_if = "is_default")]
pub(crate) consistency: VolumeConsistency,

pub(crate) struct VolumeConsistency {
    pub(crate) read_mode: ReadMode,             // default: Snapshot
    pub(crate) write_mode: WriteMode,           // default: LastWriteWins
}

pub(crate) enum ReadMode { Snapshot, Live }
pub(crate) enum WriteMode { LastWriteWins, OptimisticEtag, PessimisticLock, None }
```

Sur le scheduler de backup (champ existant `backup_schedule` étendu en sous-objet) :
```rust
pub(crate) struct BackupSchedule {
    pub(crate) cron: String,
    pub(crate) coordination: BackupCoordination,   // default: PerNode
    pub(crate) write_isolation: WriteIsolation,    // default: None
}

pub(crate) enum BackupCoordination { PerNode, MeshLeader, ManualOnly }
pub(crate) enum WriteIsolation { None, Drain, CopyOnWrite }
```

**Compatibilité** : le `backup_schedule: "0 3 * * *"` existant (string) reste accepté via `serde(untagged)` pour `BackupScheduleOrString`. Ça parse vers `BackupSchedule { cron, coordination: PerNode, write_isolation: None }` (= comportement actuel).

### D4: Pipeline d'exécution — admission filter avant `execute_guest`

Avant chaque invocation, le pipeline appelle `concurrency_admission::check(route, state).await` qui :

- `Unrestricted` → pass-through
- `NodeSingleton` → acquiert un lock local in-memory (`Mutex<HashSet<route_path>>`) ; rejette/met en attente selon `on_conflict`
- `MeshSingleton` → acquiert un `DistributedLock` ; on_conflict détermine le comportement
- `MeshLeader` → vérifie via la fonction déterministe ; si pas leader, rejette avec 503 + header `X-Tachyon-Leader: <node-id>` pour redirection client

Le guard est libéré au drop. Pour les invocations longues, le worker rafraîchit le lease toutes les `lock_ttl_ms / 2`.

### D5: Backup coordination — appel dans le scheduler existant

Avant d'appeler `volume_backup::backup_volume(...)` dans `spawn_volume_backup_scheduler`, le scheduler évalue `backup_schedule.coordination` :
- `PerNode` → exécute (comportement actuel)
- `MeshLeader` → vérifie `am_i_leader(route_path + guest_path)` ; ne fait rien sinon
- `ManualOnly` → ne fait jamais rien (scheduler skip)

Si `write_isolation == Drain`, le scheduler :
1. Acquiert un lock distribué sur la route
2. Attend que les invocations actives se terminent (`active_request_count() == 0`) avec timeout configurable
3. Rejette les nouvelles invocations pendant le backup (admission filter)
4. Lance le backup
5. Libère le lock

### D6: S3 volume commit — write_mode honoré dans `commit_s3_volumes`

`commit_s3_volumes` reçoit en argument la `WriteMode` de chaque volume :
- `LastWriteWins` → upload simple (comportement actuel)
- `OptimisticEtag` → conserve l'ETag du download initial ; au commit, conditional PUT (`If-Match: <etag>`). Sur conflit → re-télécharge, merge si possible (3-way), sinon échec
- `PessimisticLock` → utilise le `DistributedLock` sur la clé `s3-vol:<route>:<guest_path>` autour de download+execute+upload
- `None` → impossible (déjà géré par `readonly: true`)

### D7: UI — badges de risque + tooltips

Chaque option de mode a un niveau de risque (`Low | Medium | High`) calculé par combinaison :
- `Unrestricted + LastWriteWins + PerNode` → 🟢 Low (par défaut, ergonomique)
- `MeshSingleton + PessimisticLock + MeshLeader/Drain` → 🟢 Low (cohérent, plus lent)
- `Unrestricted + LastWriteWins` sur volume RW partagé → 🔴 High (perte de données silencieuse)
- `MeshLeader + OptimisticEtag` → 🟠 Medium (cohérent mais latence variable sur conflit)

Chaque badge a un tooltip avec :
- Un scénario d'échec concret (1-2 phrases)
- Le coût performance estimé
- Un lien `data-sim-scenario="..."` pour la future simulation JS

### D8: MCP — outil recommendation

`recommend_concurrency_policy({ pattern, requirements })` où :
- `pattern: "batch" | "interactive" | "stateful" | "etl" | "scheduler"`
- `requirements: { writes_shared_state?: bool, requires_ordering?: bool, max_latency_ms?: number }`

Retourne :
```json
{
  "concurrency": { "mode": "...", "on_conflict": "...", "lock_ttl_ms": ... },
  "consistency": { "read_mode": "...", "write_mode": "..." },
  "coordination": { "mode": "...", "write_isolation": "..." },
  "rationale": "...",
  "risk_level": "low|medium|high",
  "trade_offs": ["..."]
}
```

Table de décision inline (pas de LLM) :
| Pattern | Mode reco | Read | Write | Coord |
|---|---|---|---|---|
| batch | NodeSingleton + Queue | Snapshot | LastWriteWins | MeshLeader |
| interactive | Unrestricted | Snapshot | LastWriteWins | PerNode |
| stateful | MeshSingleton + Queue | Live | PessimisticLock | MeshLeader + Drain |
| etl | MeshLeader | Snapshot | OptimisticEtag | MeshLeader |
| scheduler | MeshLeader + Reject | Snapshot | LastWriteWins | MeshLeader |

## Risks / Trade-offs

- **Lock distribué best-effort** : fenêtre de course si la sync inter-node a du lag. Mitigation : TTL ajustable + on_conflict pour gérer les fausses acquisitions
- **Élection déterministe pendant un node failover** : pendant la propagation du changement de node registry (~quelques secondes), deux nodes peuvent se croire leader. Acceptable pour backups (idempotents) et schedulers. Documenté
- **Latence des modes synchronisés** : `MeshSingleton + Queue` peut introduire des secondes d'attente. C'est le but : l'utilisateur opt-in en connaissance de cause
- **Complexité UX** : 3 dimensions × 3-4 valeurs = ~36 combinaisons. Mitigation : outil MCP recommande, UI montre seulement les combos cohérents (cache les combos invalides comme `Unrestricted + PessimisticLock` qui n'a pas de sens)

## Migration Plan

1. Ajouter les types (enums + structs) avec `Default` impl = comportement actuel
2. Étendre `IntegrityRoute` et `IntegrityVolume` avec `#[serde(default, skip_serializing_if = ...)]`
3. Implémenter `DistributedLock` (table redb + API) — testable isolément
4. Implémenter `leader_election::am_i_leader(key)` — testable isolément
5. Implémenter `concurrency_admission::check()` — intégrer dans `execute_guest`
6. Étendre `commit_s3_volumes` avec `write_mode`
7. Étendre `spawn_volume_backup_scheduler` avec `coordination` et `write_isolation`
8. Ajouter l'outil MCP et la fonction tachyon-client
9. Ajouter le panneau UI avec badges de risque
10. Documenter dans README + AGENTS.md le tableau de décision

## Open Questions

- Faut-il exposer `lock_ttl_ms` à l'utilisateur ou le calculer dynamiquement à partir de la timeout de la route ? → **Exposé en phase 1** pour permettre l'expérimentation, défaut sensé (30s)
- Le `MeshLeader` doit-il offrir une garantie "exactly-once" ou "at-most-one-at-a-time" ? → **at-most-one-at-a-time** seulement (élection déterministe), exactly-once nécessiterait un consensus type Raft
- Faut-il rendre disponible la primitive `DistributedLock` aux WIT guests (via le bridge) ? → **Non en phase 1**, c'est un mécanisme host-only. Phase 2 si demandé via une WIT interface dédiée

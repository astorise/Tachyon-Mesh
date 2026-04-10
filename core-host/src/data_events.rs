use crate::store::{CoreStore, CoreStoreBucket};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const CLAIM_LEASE_MS: u64 = 30_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OutboxEvent {
    pub(crate) id: String,
    pub(crate) content_type: String,
    pub(crate) body: Vec<u8>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct StoredOutboxTable {
    #[serde(default)]
    events: Vec<StoredOutboxEvent>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoredOutboxEvent {
    id: String,
    #[serde(default)]
    content_type: String,
    #[serde(default)]
    body: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    claimed_at_ms: Option<u64>,
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn enqueue_event(
    core_store: &CoreStore,
    db_url: &str,
    table: &str,
    body: Vec<u8>,
    content_type: impl Into<String>,
) -> Result<String> {
    let key = table_key(db_url, table)?;
    let mut stored = load_table(core_store, &key)?;
    let id = Uuid::new_v4().to_string();
    stored.events.push(StoredOutboxEvent {
        id: id.clone(),
        content_type: content_type.into(),
        body,
        claimed_at_ms: None,
    });
    save_table(core_store, &key, &stored)?;
    Ok(id)
}

pub(crate) fn claim_events(
    core_store: &CoreStore,
    db_url: &str,
    table: &str,
    max_events: u32,
) -> Result<Vec<OutboxEvent>> {
    if max_events == 0 {
        return Ok(Vec::new());
    }

    let key = table_key(db_url, table)?;
    let mut stored = load_table(core_store, &key)?;
    let now = unix_time_ms();
    let mut claimed = Vec::new();
    for event in &mut stored.events {
        let available = event
            .claimed_at_ms
            .is_none_or(|claimed_at_ms| now.saturating_sub(claimed_at_ms) >= CLAIM_LEASE_MS);
        if !available {
            continue;
        }

        event.claimed_at_ms = Some(now);
        claimed.push(OutboxEvent {
            id: event.id.clone(),
            content_type: event.content_type.clone(),
            body: event.body.clone(),
        });
        if claimed.len() >= max_events as usize {
            break;
        }
    }
    save_table(core_store, &key, &stored)?;
    Ok(claimed)
}

pub(crate) fn ack_event(core_store: &CoreStore, db_url: &str, table: &str, id: &str) -> Result<()> {
    let key = table_key(db_url, table)?;
    let mut stored = load_table(core_store, &key)?;
    stored.events.retain(|event| event.id != id);
    save_table(core_store, &key, &stored)
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn pending_count(core_store: &CoreStore, db_url: &str, table: &str) -> Result<usize> {
    let key = table_key(db_url, table)?;
    Ok(load_table(core_store, &key)?.events.len())
}

fn table_key(db_url: &str, table: &str) -> Result<String> {
    let db_url = normalize_non_empty("DB_URL", db_url)?;
    let table = normalize_non_empty("OUTBOX_TABLE", table)?;
    Ok(format!("{db_url}::{table}"))
}

fn normalize_non_empty(label: &str, value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        anyhow::bail!("{label} must not be empty");
    }
    Ok(trimmed.to_owned())
}

fn load_table(core_store: &CoreStore, key: &str) -> Result<StoredOutboxTable> {
    let Some(payload) = core_store
        .get(CoreStoreBucket::DataEvents, key)
        .with_context(|| format!("failed to read outbox table `{key}` from core store"))?
    else {
        return Ok(StoredOutboxTable::default());
    };
    serde_json::from_slice(&payload)
        .with_context(|| format!("failed to decode outbox table `{key}`"))
}

fn save_table(core_store: &CoreStore, key: &str, table: &StoredOutboxTable) -> Result<()> {
    let payload = serde_json::to_vec(table)
        .with_context(|| format!("failed to encode outbox table `{key}`"))?;
    core_store
        .put(CoreStoreBucket::DataEvents, key, &payload)
        .with_context(|| format!("failed to persist outbox table `{key}`"))
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::CoreStore;

    fn open_test_store() -> CoreStore {
        let path = std::env::temp_dir().join(format!("tachyon-outbox-{}.db", Uuid::new_v4()));
        CoreStore::open(&path).expect("test outbox store should open")
    }

    #[test]
    fn claim_and_ack_round_trip_outbox_events() {
        let store = open_test_store();
        let event_id = enqueue_event(
            &store,
            "outbox://integration",
            "events_outbox",
            br#"{"kind":"created"}"#.to_vec(),
            "application/json",
        )
        .expect("event should enqueue");

        let claimed = claim_events(&store, "outbox://integration", "events_outbox", 8)
            .expect("events should claim");
        assert_eq!(
            claimed,
            vec![OutboxEvent {
                id: event_id.clone(),
                content_type: "application/json".to_owned(),
                body: br#"{"kind":"created"}"#.to_vec(),
            }]
        );
        assert_eq!(
            pending_count(&store, "outbox://integration", "events_outbox")
                .expect("pending count should be readable"),
            1
        );

        ack_event(&store, "outbox://integration", "events_outbox", &event_id)
            .expect("event should ack");
        assert_eq!(
            pending_count(&store, "outbox://integration", "events_outbox")
                .expect("pending count should be readable"),
            0
        );
    }

    #[test]
    fn claim_skips_non_expired_leases() {
        let store = open_test_store();
        let key = table_key("outbox://integration", "events_outbox").expect("key should build");
        save_table(
            &store,
            &key,
            &StoredOutboxTable {
                events: vec![StoredOutboxEvent {
                    id: "evt-1".to_owned(),
                    content_type: "application/json".to_owned(),
                    body: br#"{}"#.to_vec(),
                    claimed_at_ms: Some(unix_time_ms()),
                }],
            },
        )
        .expect("table should persist");

        assert!(
            claim_events(&store, "outbox://integration", "events_outbox", 8)
                .expect("claim should succeed")
                .is_empty()
        );
    }
}

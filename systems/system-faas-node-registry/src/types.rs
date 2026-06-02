use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GpuStats {
    pub id: String,
    pub model: String,
    pub vram_total_mb: u64,
    pub vram_used_mb: u64,
    pub compute_utilization: f64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ActiveSystem {
    pub slug: String,
    pub version: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NodeCapabilities {
    pub total_ram_mb: u64,
    pub available_ram_mb: u64,
    pub accelerators: Vec<String>,
    pub gpus: Vec<GpuStats>,
    pub active_systems: Vec<ActiveSystem>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EnrolledNode {
    pub node_id: String,
    pub public_key: String,
    pub status: String,
    pub approved_at: u64,
    pub last_seen: u64,
    pub region: Option<String>,
    pub zone: Option<String>,
    pub capabilities: NodeCapabilities,
    /// Approval provenance: `pin:<operator>` or `oidc:<subject>`. Defaults empty
    /// for records written before zero-touch enrollment existed.
    #[serde(default)]
    pub approved_by: String,
    /// The `auto_approve_tags` matchers that authorized an automatic approval.
    /// Empty for PIN approvals.
    #[serde(default)]
    pub approval_tags: Vec<String>,
}

impl EnrolledNode {
    pub fn awaiting_capabilities(node_id: String, public_key: String, now: u64) -> Self {
        Self::awaiting_capabilities_with_provenance(
            node_id,
            public_key,
            now,
            String::new(),
            Vec::new(),
        )
    }

    pub fn awaiting_capabilities_with_provenance(
        node_id: String,
        public_key: String,
        now: u64,
        approved_by: String,
        approval_tags: Vec<String>,
    ) -> Self {
        Self {
            node_id,
            public_key,
            status: "awaiting-capabilities".to_owned(),
            approved_at: now,
            last_seen: now,
            region: None,
            zone: None,
            capabilities: NodeCapabilities::default(),
            approved_by,
            approval_tags,
        }
    }
}

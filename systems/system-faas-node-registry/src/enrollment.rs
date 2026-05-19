use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EnrollmentSession {
    pub session_id: String,
    pub node_public_key: String,
    pub pin: String,
    pub created_at: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EnrollmentOutcome {
    pub session_id: String,
    pub approved: bool,
    pub node_id: Option<String>,
}

pub fn deterministic_pin(seed: &str) -> String {
    let mut acc = 0_u32;
    for byte in seed.bytes() {
        acc = acc.wrapping_mul(31).wrapping_add(byte as u32);
    }
    format!("{:06}", acc % 1_000_000)
}

pub fn node_id_from_public_key(public_key: &str) -> String {
    let trimmed = public_key.trim();
    let suffix: String = trimmed
        .chars()
        .rev()
        .take(8)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    if suffix.is_empty() {
        "node-unknown".to_owned()
    } else {
        format!("node-{suffix}")
    }
}

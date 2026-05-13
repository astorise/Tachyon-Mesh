use super::*;

pub(crate) use store::GraphEdge;

/// Host-side handle for a `graph::workspace-graph` WIT resource.
/// Stored in `ComponentHostState::table` and dropped automatically
/// when the Wasm guest releases the handle, which prevents redb
/// reader exhaustion on long-running FaaS invocations.
#[allow(dead_code)]
pub(crate) struct WorkspaceGraphResource {
    pub(crate) graph_name: String,
    pub(crate) core_store: Arc<store::CoreStore>,
}

use super::*;

pub(crate) use store::GraphEdge;

/// Host-side handle for a `graph::workspace-graph` WIT resource.
/// Stored in `ComponentHostState::table` and dropped automatically
/// when the Wasm guest releases the handle, which prevents redb
/// reader exhaustion on long-running FaaS invocations.
///
/// `#[allow(dead_code)]`: the struct is constructed inside the WIT
/// `HostWorkspaceGraph::new()` host binding and stored in a
/// `wasmtime::component::ResourceTable`, which is opaque to the
/// dead-code lint. This is NOT an experimental-feature placebo.
#[allow(dead_code)]
pub(crate) struct WorkspaceGraphResource {
    pub(crate) graph_name: String,
    pub(crate) core_store: Arc<store::CoreStore>,
}

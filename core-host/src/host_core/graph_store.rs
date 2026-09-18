use super::*;

pub(crate) use store::GraphEdge;

/// Host-side handle for a `graph::workspace-graph` WIT resource.
/// Stored in `ComponentHostState::table` and dropped automatically
/// when the Wasm guest releases the handle, which prevents redb
/// reader exhaustion on long-running FaaS invocations.
///
/// `wasmtime::component::ResourceTable<T>` is a typed table — `get`/`get_mut`
/// return an ordinary `&T`/`&mut T`, not a type-erased handle — so every
/// field this struct's own methods read back is visible to the dead-code
/// lint like any other field access; no dead-code allow is needed.
pub(crate) struct WorkspaceGraphResource {
    pub(crate) graph_name: String,
    pub(crate) core_store: Arc<store::CoreStore>,
    /// Set when the graph constructor was denied by deployment scopes. Every
    /// subsequent method on this handle returns this error immediately.
    pub(crate) scope_denial: Option<String>,
}

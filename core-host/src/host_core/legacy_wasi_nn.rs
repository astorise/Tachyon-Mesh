//! Legacy (pre-Component) WASI-NN plumbing.
//!
//! This module exists solely to satisfy the `wasi_ephemeral_nn` imports of
//! the old-style, non-Component `guest-ai` WASM module — the ABI predating
//! Magnetar's Component invocation contract. It is wired only into
//! [`super::runtime_types::LegacyHostState`] via
//! [`super::guest_runtime::build_linker`]; the modern Component host state
//! (`ComponentHostState`) and the Magnetar bridge in `component_hosts.rs`
//! never reference it. No inference Component's weights, graph, or output
//! ever passes through this registry — production builds always wire an
//! empty one.

use wasmtime_wasi_nn::{
    Graph as WasiGraph, GraphRegistry, Registry as WasiRegistry, witx::WasiNnCtx,
};

// Only `build_legacy_wasi_nn_ctx`'s `#[cfg(not(test))]` arm constructs
// this, so a `cfg(test)` compilation (`--all-targets`, any feature
// combination) sees it as dead — same reasoning as `component_aliases`'s
// `cfg_attr` just below.
#[cfg_attr(test, allow(dead_code))]
struct EmptyGraphRegistry;

impl GraphRegistry for EmptyGraphRegistry {
    fn get(&self, _name: &str) -> Option<&WasiGraph> {
        None
    }

    fn get_mut(&mut self, _name: &str) -> Option<&mut WasiGraph> {
        None
    }
}

#[cfg(test)]
struct MockPreloadedGraphRegistry {
    graphs: std::collections::HashMap<String, WasiGraph>,
}

#[cfg(test)]
impl MockPreloadedGraphRegistry {
    fn from_aliases(aliases: impl IntoIterator<Item = String>) -> Self {
        use wasmtime_wasi_nn::{
            ExecutionContext, Graph,
            backend::{BackendError, BackendExecutionContext, BackendGraph, Id, NamedTensor},
            wit::{Tensor as WasiTensor, TensorType as WasiTensorType},
        };

        struct MockGraph;
        struct MockCtx;

        impl BackendGraph for MockGraph {
            fn init_execution_context(&self) -> Result<ExecutionContext, BackendError> {
                Ok(ExecutionContext::from(
                    Box::new(MockCtx) as Box<dyn BackendExecutionContext>
                ))
            }
        }

        impl BackendExecutionContext for MockCtx {
            fn set_input(&mut self, _id: Id, _tensor: &WasiTensor) -> Result<(), BackendError> {
                Ok(())
            }

            fn compute(
                &mut self,
                _named: Option<Vec<NamedTensor>>,
            ) -> Result<Option<Vec<NamedTensor>>, BackendError> {
                Ok(None)
            }

            fn get_output(&mut self, _id: Id) -> Result<WasiTensor, BackendError> {
                Ok(WasiTensor {
                    dimensions: vec![MOCK_LEGACY_RESPONSE.len() as u32],
                    ty: WasiTensorType::U8,
                    data: MOCK_LEGACY_RESPONSE.as_bytes().to_vec(),
                })
            }
        }

        let graphs = aliases
            .into_iter()
            .map(|alias| {
                let graph = Graph::from(Box::new(MockGraph) as Box<dyn BackendGraph>);
                (alias, graph)
            })
            .collect();
        Self { graphs }
    }
}

#[cfg(test)]
const MOCK_LEGACY_RESPONSE: &str = "MOCK_COMPONENT_RESPONSE";

#[cfg(test)]
impl GraphRegistry for MockPreloadedGraphRegistry {
    fn get(&self, name: &str) -> Option<&WasiGraph> {
        self.graphs.get(name)
    }

    fn get_mut(&mut self, name: &str) -> Option<&mut WasiGraph> {
        self.graphs.get_mut(name)
    }
}

/// Builds the legacy `guest-ai` module's WASI-NN context. `component_aliases`
/// is used only under `#[cfg(test)]`, to preload mock graphs keyed by the
/// same aliases the Magnetar Component registry happens to use in tests —
/// production wires an always-empty registry regardless of what's loaded.
pub(crate) fn build_legacy_wasi_nn_ctx(
    #[cfg_attr(not(test), allow(unused_variables))] component_aliases: Vec<String>,
) -> WasiNnCtx {
    #[cfg(test)]
    let registry = WasiRegistry::from(MockPreloadedGraphRegistry::from_aliases(component_aliases));
    #[cfg(not(test))]
    let registry = WasiRegistry::from(EmptyGraphRegistry);
    WasiNnCtx::new([], registry)
}

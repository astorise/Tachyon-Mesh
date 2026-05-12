//! Official Rust SDK surface for Tachyon Mesh FaaS guests.
//!
//! Release automation regenerates bindings from `sdk/wit/tachyon.wit`.

pub const SDK_VERSION: &str = env!("CARGO_PKG_VERSION");

// Guest-side bindings for the kv-partition interface, generated at compile time
// by wit-bindgen from the `kv-consumer` world in `sdk/wit/tachyon.wit`.
mod bindings {
    wit_bindgen::generate!({
        world: "kv-consumer",
        path: "../wit",
    });
}

/// Ergonomic wrappers for the `tachyon:mesh/kv-partition` WIT interface.
///
/// # Example
/// ```ignore
/// use tachyon_faas_sdk::ai::kv_partition::Table;
///
/// let context_table = Table::new("viking-context");
/// context_table.batch_set(&[
///     ("node:1".to_string(), b"data1".to_vec()),
///     ("node:2".to_string(), b"data2".to_vec()),
/// ]).unwrap();
/// let history = context_table
///     .get_range("timestamp:20260512T0000", "timestamp:20260512T2359", 50, 0)
///     .unwrap();
/// ```
pub mod ai {
    pub mod kv_partition {
        use crate::bindings::tachyon::mesh::kv_partition;

        /// A handle to a named key-value partition backed by redb on the host.
        ///
        /// The Wasm runtime automatically calls the resource `drop` export
        /// when this value goes out of scope, releasing the host-side tracking
        /// entry without any explicit close call.
        pub struct Table {
            inner: kv_partition::Table,
        }

        impl Table {
            /// Opens or creates the named KV partition on the host.
            pub fn new(name: &str) -> Self {
                Self {
                    inner: kv_partition::Table::new(name),
                }
            }

            /// Reads the value stored at `key`. Returns `Err` if the key does
            /// not exist or a storage failure occurs.
            pub fn get(&self, key: &str) -> Result<Vec<u8>, String> {
                self.inner.get(key)
            }

            /// Inserts or replaces the value at `key`.
            pub fn set(&self, key: &str, value: &[u8]) -> Result<(), String> {
                self.inner.set(key, value)
            }

            /// Removes the entry at `key`. No-op if the key is absent.
            pub fn delete(&self, key: &str) -> Result<(), String> {
                self.inner.delete(key)
            }

            /// Atomically inserts all `entries` within a single redb write
            /// transaction. Prefer this over repeated `set` calls for bulk loads.
            pub fn batch_set(&self, entries: &[(String, Vec<u8>)]) -> Result<(), String> {
                self.inner.batch_set(entries)
            }

            /// Returns up to `limit` entries whose keys fall in
            /// `[start_key, end_key)`, skipping the first `offset` matches.
            /// Pagination is delegated to redb's native B-Tree iterator so no
            /// full table scan crosses the Wasm–host IPC boundary.
            pub fn get_range(
                &self,
                start_key: &str,
                end_key: &str,
                limit: u32,
                offset: u32,
            ) -> Result<Vec<(String, Vec<u8>)>, String> {
                self.inner.get_range(start_key, end_key, limit, offset)
            }
        }
    }
}

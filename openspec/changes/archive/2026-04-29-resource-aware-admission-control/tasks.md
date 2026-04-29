# Tasks: Admission Control Implementation

**Agent Instruction:** Implement resource barriers in the request lifecycle to prevent node saturation.

- [x] **UI Schema:** Update `tachyon-ui/gen/schemas/capabilities.json` to include `min_ram_gb` and `admission_strategy` fields.
- [x] **Host Telemetry:** Create a helper function in `core-host/src/resiliency.rs` that returns available system RAM using the `sysinfo` crate.
- [x] **Routing Guard:** Inject a resource verification check into the HTTP/3 request pipeline before calling `wasmtime` instantiation.
- [x] **Mesh Proxying:** Connect the local failure path to the `system-faas-mesh-overlay` to dynamically route the request to the most capable neighboring node IP.
- [x] **UI Alerts:** Add a notification component in Tachyon-UI that reacts to `system-faas-buffer` load events.

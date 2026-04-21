# Tasks: Change 067-068 Implementation

**Agent Instruction:** Implement the unified connection and authentication pipeline. Maintain strict 4-space indentation for injected code to avoid Markdown parsing errors.

- [x] Define the `tachyon:identity/auth` WIT contract and add the `system-faas-auth` component crate to the workspace build.
- [x] Implement JWT verification plus recovery-code primitives in `system-faas-auth` with `wit-bindgen`.
- [x] Add `/admin/status` plus Bearer-token middleware in `core-host` that invokes `system-faas-auth` before serving admin traffic.
- [x] Add a global connection `RwLock` in `tachyon-client` and implement `set_connection(url, token, cert)` with remote validation.
- [x] Inject the connection overlay in `tachyon-ui`, wire `connect_to_node`, read the optional identity file, and fade the overlay out with GSAP after a successful connection.
- [x] Update CI and container packaging so `system-faas-auth` is built and shipped with the runtime artifacts.

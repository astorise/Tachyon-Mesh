# Tasks: Change 027 Implementation

**Agent Instruction:** Read the `proposal.md` and the delta spec under `specs/`. Implement the credential graph validation and the middleware execution logic.

- [x] Update `tachyon-cli` and `core-host` target schemas to support `requires_credentials` and optional `middleware`.
- [x] Extend startup dependency validation so callers must declare every credential required by their resolved dependencies.
- [x] Execute configured middleware targets before the main target and short-circuit non-200 responses.
- [x] Validate the middleware flow and credential-delegation startup failure with end-to-end tests.

## Validation Notes
1. Create a `system-faas-auth` module that returns `403` when `X-Token` is missing and `200` when it is present.
2. Configure `FaaS-A` to use that middleware and verify unauthorized requests return `403` before `FaaS-A` executes.
3. Add dependency `FaaS-A -> FaaS-B`, require credential `c2` on `FaaS-B`, and verify startup fails until `FaaS-A` also declares `c2`.

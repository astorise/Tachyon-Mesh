## Context

The native web component shell has accumulated domain dashboards but still has wiring gaps: some existing routes are only rendered as static panels, the overview dashboard uses mock counters, and the IAM component has no direct operator staging form for invoking `stage_signup`.

## Goals / Non-Goals

**Goals:**
- Route `registry` and `topology` through `ComponentRegistry`.
- Fetch overview telemetry from the Tauri `get_mesh_graph` command and animate counters after data arrives.
- Add a Stage New Operator form to `<tachyon-iam>` that invokes `stage_signup`.

**Non-Goals:**
- Add new Rust telemetry commands.
- Implement full admin user CRUD.
- Replace the existing invite-token enrollment flow.

## Decisions

- Use the existing mesh graph snapshot shape: `routes`, `batchTargets`, and route target counts.
- Derive utilization from graph features until a dedicated hardware utilization command is introduced.
- Keep the IAM staging form separate from the existing first-run enrollment steps so both workflows can coexist.

## Risks / Trade-offs

- Derived GPU utilization is an approximation. Mitigation: the panel is structured so a future command can replace the calculation without changing the DOM contract.
- `stage_signup` still requires an enrollment token and URL. Mitigation: the form exposes both fields and dispatches global toast notifications on success/failure.

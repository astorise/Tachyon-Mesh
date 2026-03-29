# Specifications: System Telemetry FaaS Foundations

- Add sealed route roles to `integrity.lock` so `core-host` can distinguish normal and privileged guests.
- Introduce a dedicated `system-faas-guest` WIT world with a telemetry snapshot import.
- Execute a Prometheus-formatted system guest only when the sealed route is marked `system`.
- Shed system route execution under high active-request pressure.

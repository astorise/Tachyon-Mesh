# Security Policy

Tachyon-Mesh orchestrates AI and FaaS workloads with admin credentials, mTLS
identities, and access to whatever the mesh is deployed to route — a
vulnerability here can have real consequences. Please report it privately.

## Reporting a vulnerability

Use GitHub's private vulnerability reporting for this repository:

**[Report a vulnerability](https://github.com/astorise/tachyon-mesh/security/advisories/new)**
(Security tab → "Report a vulnerability")

This opens a private advisory visible only to maintainers, so a report never
sits in the public issue tracker while it's unpatched. If you cannot use
GitHub's reporting flow, open a regular issue asking a maintainer to contact
you, without any exploit details in the issue body itself.

Please include, as available:

- The affected version, commit, or release artifact (`core-host`, `tachyon-ui`,
  `tachyon-mcp`, an SDK, a specific `system-faas-*` guest, etc.).
- Steps to reproduce, or a proof-of-concept.
- The impact you'd expect (credential exposure, privilege escalation, RCE,
  denial of service, etc.).

## What's in scope

- `core-host` and its `/admin/*` and data-plane HTTP/WebSocket/L4 surfaces.
- The authn/authz WASM components and the WIT contracts they implement.
- `tachyon-ui` (the Tauri desktop app) and `tachyon-client`.
- `tachyon-mcp` and the `system-faas-*` guest modules shipped in this repo.
- The release/build pipeline (`.github/workflows/`, `Dockerfile*`,
  `scripts/get-tachyon.*`) and supply-chain integrity (checksums, signing).

Vulnerabilities in third-party dependencies should generally be reported
upstream, but let us know too if one is exploitable through Tachyon-Mesh's
own use of it — `cargo audit` / `cargo deny` findings tracked in `deny.toml`
are already on our radar and don't need a separate report.

## Supported versions

This project does not yet maintain long-term-supported release branches.
Security fixes are made against the latest release; please upgrade before
reporting an issue that may already be fixed.

## Response

We aim to acknowledge new reports within a few business days and to keep you
updated as a fix is developed. Coordinated disclosure timing is negotiable —
tell us what you need.

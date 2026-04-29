# Proposal: Outbound Device Flow Enrollment

## Context
New Tachyon Edge nodes often start behind strict firewalls or NATs, preventing direct inbound connections from an administrator's UI. Furthermore, keeping the `core-host` minimal means we should not bloat it with the complex state machine required for pairing and certificate exchange.

## Proposed Solution
We will implement the enrollment process via a dedicated, ephemeral System FaaS: `system-faas-enrollment`.
1. **Device Flow (PIN):** Upon first boot (when no valid `integrity.lock` or mTLS cert exists), the host instantiates `system-faas-enrollment`. This module generates a short, temporary PIN (e.g., `A7X-92B`).
2. **Outbound Connection:** The FaaS initiates a long-lived outbound HTTP/3 or WebSocket connection to a known cluster endpoint (or load balancer). It transmits its public key and waits.
3. **UI Approval:** The administrator connects Tachyon Studio (UI) to *any* active node in the masterless mesh and enters the PIN.
4. **Certificate Injection:** The active node signs the new node's public key and sends the certificate down the already-open outbound tunnel. The `system-faas-enrollment` saves the credentials, terminates itself, and triggers the `core-host` to load the full `mesh-overlay` and configuration.

## Objectives
- Bypass inbound firewall restrictions seamlessly (NAT-friendly).
- Provide a smooth "WebLogic/Smart TV" pairing experience for the administrator.
- Isolate the enrollment logic in a System FaaS to maintain a lean `core-host`.
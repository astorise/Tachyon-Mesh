## ADDED Requirements

### Requirement: Layer 4 bridge allocation is steered to the least-loaded capable node before traffic starts
The control plane SHALL consider bridge load and public reachability when deciding which node should host a newly requested Layer 4 bridge.

#### Scenario: The local node is saturated when a bridge is requested
- **WHEN** a bridge allocation request arrives and the local node is above the bridge load threshold
- **THEN** the control plane forwards allocation to a healthier peer
- **AND** returns the actual public endpoint of the peer that will host the bridge

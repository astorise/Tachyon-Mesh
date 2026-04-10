# Global L4 Bridge Steering

## Purpose
Define how Tachyon steers new Layer 4 bridge allocations toward a healthier node before media traffic starts, while preserving the same bridge-controller abstraction for user guests.

## Requirements
### Requirement: Layer 4 bridge allocation is steered to the least-loaded capable node before traffic starts
The control plane SHALL consider local bridge load and public reachability when deciding which node should host a newly requested Layer 4 bridge.

#### Scenario: The local node is saturated when a bridge is requested
- **WHEN** a bridge allocation request arrives and the local node is above the bridge load threshold
- **THEN** the control plane forwards allocation to a healthier peer
- **AND** returns the actual public endpoint of the peer that will host the bridge

### Requirement: Bridge telemetry advertises steering signals through system gossip
The platform SHALL publish live Layer 4 bridge telemetry through the telemetry and gossip interfaces so delegated bridge decisions can be made without inspecting the data plane directly.

#### Scenario: A node reports relay saturation
- **WHEN** the host samples bridge activity
- **THEN** it exposes active relay count, throughput, load score, and advertised public IP through `telemetry-reader`
- **AND** the gossip system route broadcasts that state to peer nodes

### Requirement: Delegated bridge allocations preserve the user-facing contract
The platform SHALL keep bridge delegation transparent to user guests by returning the selected node IP and allocated ports through the existing bridge controller API.

#### Scenario: A user guest starts a bridged call
- **WHEN** the guest requests a bridge through `bridge-controller.create-bridge`
- **THEN** the host or delegated peer returns the bridge identifier, public IP, and relay ports
- **AND** the guest can inject that endpoint into its call setup response without knowing whether allocation happened locally or remotely

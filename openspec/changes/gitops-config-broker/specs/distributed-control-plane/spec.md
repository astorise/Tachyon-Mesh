# distributed-control-plane Delta

## ADDED Requirements

### Requirement: Configuration MUST be managed via GitOps Multi-branching
The control plane SHALL map node environments to Git branches so that configuration changes can be promoted natively and rolled back atomically.

#### Scenario: Promoting a configuration to staging
- **GIVEN** a node tagged with `env=staging`
- **WHEN** the `system-faas-config-api` receives a promotion request from `dev` to `staging`
- **THEN** the `gitops-broker` performs a merge into the `staging` branch
- **AND** broadcasts the new commit hash via gossip to all `staging` nodes

### Requirement: Nodes MUST support offline-first Fast-Boot
Every node SHALL rely on a local WASI persistent volume for its configuration store to ensure instantaneous startup without remote dependencies.

#### Scenario: A node restarts without network connectivity
- **GIVEN** an Edge node isolated from the internet
- **WHEN** the `core-host` daemon restarts
- **THEN** the `gitops-broker` reads the last known configuration from the local WASI volume
- **AND** the data-plane routes traffic immediately using this state
- **AND** defers S3 synchronization until the network is restored

### Requirement: Configuration updates MUST be Event-Driven and Async
Internal configuration updates SHALL NOT use synchronous Host Calls that block the data-plane, but MUST use an asynchronous Pub/Sub broadcast channel.

#### Scenario: Hot-reloading a component
- **WHEN** the `gitops-broker` checks out a new commit
- **THEN** it emits a `ConfigUpdate` event to the host
- **AND** listening components (e.g., Rate Limiter, Router) consume the event to swap their state atomically without dropping active connections.

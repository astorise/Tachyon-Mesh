# ai-orchestration Specification Delta

## ADDED Requirements

### Requirement: VRAM Priority Tiers
The AI inference host SHALL assign each safetensors layer residency a VRAM priority of `Active`, `Hot`, or `Volatile`.

#### Scenario: Volatile prewarm memory is reclaimed for live inference
- **GIVEN** a predictive LoRA adapter is resident in VRAM with `Volatile` priority
- **WHEN** a live inference request needs VRAM for `Active` tensors
- **THEN** the host SHALL evict the volatile residency before failing the live request for insufficient VRAM
- **AND** the live request SHALL keep its `Active` residency protected from speculative evictions

#### Scenario: Expired hot memory is reclaimed after volatile memory
- **GIVEN** both `Volatile` and expired `Hot` safetensors allocations are resident in VRAM
- **WHEN** reclaiming volatile memory alone is insufficient for an active request
- **THEN** the host SHALL reclaim expired `Hot` allocations before returning an out-of-memory error

### Requirement: Predictive Broker Prewarms Tenant LoRA
The model broker SHALL translate auth session CDC events into volatile layer prewarm instructions for the tenant default LoRA adapter.

#### Scenario: Auth session creation schedules volatile prewarm
- **GIVEN** the broker receives an auth session mutation event with operation `insert`
- **AND** the event payload contains a tenant identifier
- **WHEN** the event belongs to the auth session namespace
- **THEN** the broker SHALL resolve the tenant default LoRA model id
- **AND** return a layer load instruction with priority `volatile`

#### Scenario: Non-auth mutations are ignored
- **GIVEN** the broker receives a mutation event outside the auth namespace
- **WHEN** evaluating predictive prewarm eligibility
- **THEN** the broker SHALL NOT produce a layer load instruction

### Requirement: Dynamic VRAM TTL From Time-Series Heuristics
The model broker SHALL calculate a dynamic volatile VRAM TTL from tenant prompt-history density for the current hour.

#### Scenario: High follow-up probability extends volatile TTL
- **GIVEN** tenant prompt history shows a follow-up probability greater than `0.8`
- **WHEN** a prompt finishes
- **THEN** the broker SHALL select a volatile VRAM TTL of `1800` seconds

#### Scenario: Standard follow-up probability keeps default TTL
- **GIVEN** tenant prompt history shows a follow-up probability less than or equal to `0.8`
- **WHEN** a prompt finishes
- **THEN** the broker SHALL select the standard volatile VRAM TTL of `300` seconds

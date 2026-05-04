# resource-quotas Delta

## ADDED Requirements

### Requirement: Wasm instances MUST be bounded by declarative Compute Quotas
The Memory Governor and MicroVM runner SHALL enforce CPU and Memory restrictions based on the `ComputeQuota` attached to a specific Target Group.

#### Scenario: Updating a memory limit dynamically
- **WHEN** the config API updates the `max_memory_mb` for a specific `target_group_ref`
- **THEN** the Memory Governor applies this new threshold to all newly spawned instances of that component
- **AND** gracefully recycles existing instances that exceed the new limit according to the Zero-Downtime policy.

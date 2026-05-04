# identity-and-security-suite Delta

## ADDED Requirements

### Requirement: Security Configuration MUST use a typed WIT contract
The configuration API SHALL validate all IAM and Rate-Limiting intents against a strict `config-security.wit` interface.

#### Scenario: Validating a new tenant quota
- **WHEN** the `system-faas-config-api` receives a `RateLimitPolicy`
- **THEN** it validates the structure and enum mappings (e.g., `distributed_crdt`)
- **AND** rejects invalid payload types with a clean error string, preserving Zero-Panic operations.

### Requirement: Rate Limiting MUST be linkable to Identity Claims
The declarative schema SHALL allow operators to define rate limits scoped to specific identity attributes (like `tenant_id`) extracted by the Authentication providers.

#### Scenario: Tenant-aware CRDT limiting
- **GIVEN** a valid `SecurityConfiguration` with `scope: identity_tenant`
- **WHEN** requests arrive with varying `tenant_id` JWT claims
- **THEN** the distributed CRDT rate limiter maintains separate threshold counters for each distinct tenant.

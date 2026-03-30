## ADDED Requirements

### Requirement: Component guests retrieve secrets through a typed host vault import
The workspace SHALL define a `secrets-vault` WIT interface in `wit/tachyon.wit`, and `core-host` SHALL implement that import for `faas-guest` components without exposing the same secrets through the guest environment block.

#### Scenario: Vault is disabled at compile time
- **WHEN** `core-host` runs without `--features secrets-vault`
- **THEN** a `faas-guest` component can still call `get-secret("DB_PASS")`
- **AND** the host returns `vault-disabled`
- **AND** `std::env::var("DB_PASS")` inside the guest remains unset

#### Scenario: Authorized guest receives a sealed secret
- **WHEN** `core-host` is built with `--features secrets-vault`
- **AND** `/api/guest-example` is sealed with `allowed_secrets: ["DB_PASS"]`
- **THEN** the guest receives `super_secret_123` from `get-secret("DB_PASS")`
- **AND** the guest still cannot read `DB_PASS` from its environment block

#### Scenario: Unauthorized guest is denied
- **WHEN** a component guest requests a secret that is not granted by its sealed route metadata
- **THEN** the host returns `permission-denied`
- **AND** the secret value is not disclosed

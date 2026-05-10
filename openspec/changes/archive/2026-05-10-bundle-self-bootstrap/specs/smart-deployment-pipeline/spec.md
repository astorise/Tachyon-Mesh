# smart-deployment-pipeline

## MODIFIED Requirements

### Requirement: Server-Side Resolution and Locking
The core host SHALL automatically inject its own Ed25519 public key into the
`trusted_signers` list of the `IntegrityConfig` before signing and writing the
`integrity.lock`, so that subsequent reloads accept the node-signed manifest
without any manual operator step.

#### Scenario: First bundle apply self-bootstraps trusted_signers
- **GIVEN** a fresh node whose config has an empty `trusted_signers` list
- **WHEN** an admin applies a deployment bundle for the first time
- **THEN** the written `integrity.lock` contains the node's public key in
  `trusted_signers`
- **AND** the node can reload the manifest after a reboot without operator
  intervention

#### Scenario: Injection is idempotent
- **GIVEN** the node's public key is already present in `trusted_signers`
- **WHEN** a subsequent bundle apply is performed
- **THEN** `trusted_signers` contains exactly one entry for that key
- **AND** the resulting `integrity.lock` is otherwise identical to what it
  would be without the injection

## ADDED Requirements

### Requirement: Sealed routes can evaluate explicit traffic targets
The `core-host` runtime SHALL allow a sealed route to declare an ordered
`targets` array whose entries select guest artifacts by deterministic header
match or weighted rollout.

#### Scenario: Header match overrides weighted rollout
- **WHEN** a sealed route declares a target with
  `match_header { name: "X-Cohort", value: "beta" }`
- **AND** the inbound request contains `X-Cohort: beta`
- **THEN** `core-host` selects that target's `module` immediately
- **AND** it does not evaluate weighted rollout for that request

#### Scenario: Weighted rollout selects a fallback target
- **WHEN** a sealed route declares multiple targets with weights greater than
  zero
- **AND** no header-matched target applies to the inbound request
- **THEN** `core-host` selects one target using the declared weights
- **AND** guest execution uses the selected target's `module`

#### Scenario: Legacy path-derived routing remains available
- **WHEN** a sealed route omits `targets`
- **THEN** `core-host` resolves the guest module from the route path exactly as
  before

### Requirement: Host preserves cohort routing context across mesh hops
The `core-host` runtime SHALL forward cohort headers on host-managed outbound
mesh requests so a dependency chain can stay within the same rollout bucket.

#### Scenario: Mesh fetch forwards the canonical cohort header
- **WHEN** an inbound request carries either `X-Cohort` or `X-Tachyon-Cohort`
- **AND** the guest triggers a host-managed outbound mesh request
- **THEN** the outbound request includes `X-Tachyon-Cohort`
- **AND** downstream routes can evaluate the same cohort context

#### Scenario: Compatibility header is preserved for existing match rules
- **WHEN** an inbound request carries `X-Cohort`
- **AND** the host forwards a downstream mesh request
- **THEN** the outbound request also preserves `X-Cohort`
- **AND** existing header-match rules that still reference `X-Cohort` continue
  to work

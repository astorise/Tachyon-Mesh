## ADDED Requirements

### Requirement: Component bytes MAY be resolved from a digest-verified OCI artifact
A local inference binding MAY name its Component bytes with an
`oci://<registry>/<repository>:<tag>@sha256:<digest>` locator instead of a
local filesystem path. The locator's digest is the only trust identity: the
tag is a transport hint, never a substitute for it. Tachyon SHALL verify the
pulled manifest's own digest against the locator's `expected_digest` before
trusting anything the manifest declares — including its own layer digest —
and SHALL reject a tag that currently resolves to a different digest rather
than silently following it. Once resolved, the Component bytes SHALL be
materialized into the same local binding-root layout a hand-placed local
bundle already occupies, so the existing local-directory load path runs
unmodified; the Magnetar runtime adapter itself SHALL gain no OCI/registry
surface (no `pull_from_ghcr`/`login_registry`/`resolve_tag`-shaped methods).

#### Scenario: A locator resolving to its pinned digest is pulled and materialized
- **WHEN** a binding names an `oci://` locator whose registry tag currently
  resolves to the manifest digest the locator pins
- **THEN** Tachyon pulls the manifest and single layer, verifies both
  digests, and materializes the layer bytes into the binding root under the
  same fixed name a local bundle's Component file would occupy
- **AND** the existing, unmodified local-directory Component/manifest-sidecar
  discovery finds them exactly as it would a hand-placed local bundle

#### Scenario: A tag resolving to an unexpected digest is rejected
- **GIVEN** an `oci://` locator pins `expected_digest`
- **WHEN** the registry tag the locator names currently resolves to a
  different manifest digest
- **THEN** Tachyon rejects the binding with a typed error naming the digest
  mismatch
- **AND** Tachyon never falls back to trusting whatever the tag currently
  resolves to

#### Scenario: A previously-resolved digest is served from the local cache without network access
- **GIVEN** an `oci://` locator was already resolved once into the local
  content-addressed cache
- **WHEN** the same locator is resolved again
- **THEN** Tachyon serves the cached, digest-verified bytes without any
  registry request
- **AND** a cache entry whose on-disk bytes no longer match their recorded
  digest is rejected before being handed to Magnetar, not after a failed or
  garbled invocation

#### Scenario: OCI resolution is an additive, separately-gated capability
- **WHEN** `core-host` is built with `ai-inference` but without the
  `oci-artifacts` feature
- **THEN** an `oci://` locator on a binding fails closed with a typed error
  explaining the build lacks OCI artifact support
- **AND** existing `magnetar:<local path>` bindings are completely unaffected
- **AND** no OCI registry client dependency is linked into that build

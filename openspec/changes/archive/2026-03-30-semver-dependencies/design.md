# Design: Route-Level SemVer Dependencies

## Decision
Attach SemVer identity to sealed routes instead of per-request rollout targets.

Each sealed route now owns:
- a logical service `name`
- a semantic `version`
- a dependency map of `service -> VersionReq`

This keeps the public HTTP router unchanged while giving the host enough information to:
- validate the dependency graph before serving traffic
- resolve `http://tachyon/<service>` aliases to a concrete sealed route path

## Why route-level metadata
The existing `targets` array is already responsible for header-based routing and weighted rollout.
Using it for dependency resolution would make SemVer selection ambiguous because multiple rollout
targets can share one external route path. Route-level metadata avoids that ambiguity:

- public HTTP traffic still resolves by sealed route path
- dependency resolution maps a logical service name to versioned sealed routes
- internal mesh fetches are rewritten to the concrete compatible route path before the HTTP hop

## Compatibility
Older manifests remain valid:
- missing `name` defaults to the route-derived function name
- missing `version` defaults to `0.0.0`
- missing `dependencies` defaults to an empty map

## Validation
`core-host` builds a registry keyed by logical route name and sorts candidates by descending
semantic version. Startup fails when:
- a route version is invalid
- a dependency requirement is invalid
- two routes declare the same logical `name` and `version`
- a dependency requirement has no compatible loaded route

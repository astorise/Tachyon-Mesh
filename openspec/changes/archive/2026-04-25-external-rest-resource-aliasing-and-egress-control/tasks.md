# Implementation Tasks

## Phase 1: Sealed schema and validation
- [x] Extend the sealed `IntegrityConfig` payload with a top-level `resources` map using
      discriminated unions for `internal` and `external`.
- [x] Normalize and validate resource aliases, including version constraints, HTTPS targets,
      method allow-lists, and route-name collision checks.

## Phase 2: Unified outbound resolution
- [x] Reuse the existing `core-host` outbound resolution path instead of adding a separate resolver.
- [x] Resolve `http://mesh/<alias>` against sealed internal and external resources while preserving
      suffix paths and query strings.
- [x] Keep existing SemVer dependency resolution available for internal mesh dependencies.

## Phase 3: Egress controls
- [x] Block raw external outbound URLs for `user` routes unless the request goes through a sealed
      external alias.
- [x] Preserve raw outbound access for `system` routes so existing infrastructure FaaS remain
      functional.
- [x] Strip Tachyon-specific and hop-by-hop headers before external egress.
- [x] Enforce sealed method allow-lists for external aliases.

## Phase 4: Validation
- [x] **Test Case 1 (Internal):** `http://mesh/inventory-api/...` resolves to a local sealed route.
- [x] **Test Case 2 (External):** `http://mesh/payment-gateway/...` resolves to a sealed external
      HTTPS target.
- [x] **Test Case 3 (Switch):** changing a resource alias from `external` to `internal` changes the
      resolved target without guest code changes.
- [x] **Test Case 4 (Security):** raw outbound calls from `user` routes are rejected.
- [x] **Test Case 5 (Sanitization):** external egress strips Tachyon identity and routing headers.

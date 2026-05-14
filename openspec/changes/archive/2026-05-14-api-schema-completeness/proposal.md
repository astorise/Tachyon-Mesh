# Proposal: OpenAPI Completeness & Integrity Schema

## Context
The recent audit recognized the successful implementation of the dynamic OpenAPI registry (`system-faas-openapi`). However, it flagged a critical P1 documentation gap: only ~10 out of the ~34 existing HTTP routes in `core-host` are currently annotated. Furthermore, while the manifest schema is exposed, the `integrity.lock` schema is not.

## Problem
1. **Incomplete Contract:** Developers and AI agents using the Swagger UI at `/admin/docs` cannot see or interact with endpoints managing the guest lifecycle, KV store, canary routing, background workers, or chaos testing. 
2. **Offline Validation:** External tools and IDEs lack a JSON schema to validate the `integrity.lock` file syntax and structure prior to deployment.

## Proposed Solution
1. **Full Surface Annotation:** Systematically apply `#[utoipa::path]` to the remaining 24+ routes in `core-host` and register their request/response types in the `ApiDoc` components.
2. **Integrity Schema Endpoint:** Derive `JsonSchema` for the `IntegrityLock` struct and serve it at `GET /admin/schema/integrity-lock`.

## Impact
- **Trust & Parity:** The OpenAPI specification becomes the absolute, 100% accurate source of truth for the entire cluster API.
- **Agentic DX:** LLMs writing ad-hoc scripts against the Tachyon API have a complete reference.
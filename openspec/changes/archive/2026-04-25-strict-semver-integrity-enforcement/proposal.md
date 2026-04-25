# Proposal: Strict SemVer Integrity Enforcement

## Context
In a previous iteration (Change 025), we introduced Semantic Versioning (`version`) and IPC dependency constraints (`dependencies`) into the `integrity.lock` manifest. To maintain local backward compatibility, a fallback was implemented: if these fields were missing, `core-host` defaulted to `version: "0.0.0"` and an empty dependency map.

As Tachyon Mesh targets a Zero-Trust and Air-Gapped environment, maintaining this fallback prior to a v1.0 release introduces a critical security flaw: a **Downgrade Attack**. An attacker replacing the current `integrity.lock` with an older, signed version could bypass all QoS and dependency routing rules silently.

## Proposed Solution
We will apply a strict **Fail-Fast** architectural pattern. 
The `core-host` parser will no longer tolerate incomplete route metadata. The fields `version` and `dependencies` must become strictly required in the deserialization schema of the configuration payload. 

If `core-host` loads an `integrity.lock` lacking these explicit fields, it SHALL fail the integrity validation phase, log a fatal error, and abort the HTTP/3 server boot sequence.

## Objectives
- Eliminate the downgrade attack vector on the `integrity.lock` manifest.
- Enforce strict schema validation for the FaaS deployment graph.
- Clean up technical debt in `core-host` by removing default padding logic.
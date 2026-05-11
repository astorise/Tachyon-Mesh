## ADDED Requirements

### Requirement: Core host rewrites outbound secret placeholders at the WASI HTTP boundary
The core host SHALL keep real egress secrets outside guest linear memory by allowing guests to send `tachyon:secret:<uuid>` placeholders and replacing those placeholders only in the host-owned outbound HTTP path.

#### Scenario: Allowed host receives plaintext secret
- **GIVEN** a secret placeholder is registered with `api.openai.com` in its allowed hosts
- **WHEN** a guest outbound HTTP request targets `api.openai.com` with the placeholder in a header or UTF-8 body
- **THEN** the host replaces the placeholder with the plaintext secret before dispatching the request

#### Scenario: Disallowed host receives honeypot placeholder
- **GIVEN** a secret placeholder is registered only for `api.openai.com`
- **WHEN** a guest outbound HTTP request targets `evil.test` with the placeholder in a header or UTF-8 body
- **THEN** the host leaves the placeholder unchanged
- **AND** the plaintext secret is not exposed to the disallowed destination

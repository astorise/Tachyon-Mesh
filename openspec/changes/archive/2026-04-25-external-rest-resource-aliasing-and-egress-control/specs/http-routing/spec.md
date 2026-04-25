## ADDED Requirements

### Requirement: Host resolves sealed resource aliases for outbound HTTP
`core-host` SHALL resolve `http://mesh/<alias>` through the sealed `resources` map before falling
back to dependency-name routing, and SHALL preserve the suffix path and query string when it
rewrites the target.

#### Scenario: Internal resource alias rewrites to a local sealed route
- **WHEN** a guest requests `http://mesh/inventory-api/items?expand=1`
- **AND** `inventory-api` is sealed as an internal resource alias targeting a compatible route
- **THEN** the host rewrites the request to the local mesh route for that sealed target
- **AND** it preserves `/items?expand=1`

#### Scenario: External resource alias rewrites to a sealed HTTPS endpoint
- **WHEN** a guest requests `http://mesh/payment-gateway/charges?expand=1`
- **AND** `payment-gateway` is sealed as an external resource alias targeting
  `https://api.example.com/v1`
- **THEN** the host rewrites the request to `https://api.example.com/v1/charges?expand=1`
- **AND** it rejects methods that are not present in the sealed allow-list

### Requirement: Host restricts unsealed user egress
`core-host` SHALL reject raw outbound external URLs from `user` routes while still allowing
privileged `system` routes to use the existing raw infrastructure egress path.

#### Scenario: User route raw external egress is blocked
- **WHEN** a `user` route attempts to fetch `https://api.example.com/v1/ping` directly
- **THEN** the host rejects the request
- **AND** the error instructs the caller to use a sealed external resource alias

#### Scenario: External egress strips mesh-only headers
- **WHEN** the host forwards a request to an external resource alias
- **THEN** it does not forward Tachyon identity headers, hop-limit headers, or other mesh-only
  hop-by-hop routing metadata to the external destination

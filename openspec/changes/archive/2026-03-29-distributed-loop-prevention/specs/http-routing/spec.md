## MODIFIED Requirements

### Requirement: Host can fulfill mesh fetch commands emitted by a guest
If the captured guest stdout contains a single line beginning with `MESH_FETCH:`, the host SHALL interpret the remainder as an outbound HTTP target, perform the fetch on the guest's behalf, and return the fetched response body to the original client.

#### Scenario: Guest asks the host to reach a legacy service
- **WHEN** the guest stdout is `MESH_FETCH:http://legacy-service:8081/ping`
- **THEN** the host issues an outbound HTTP `GET` request to that URL
- **AND** the host returns the fetched response body as the HTTP response
- **AND** a failed outbound request results in a gateway-style error response

#### Scenario: Guest asks the host to recurse through another sealed mesh route
- **WHEN** the guest stdout is `MESH_FETCH:/api/guest-loop`
- **THEN** the host resolves the relative route against its own HTTP listener
- **AND** the host injects the decremented `X-Tachyon-Hop-Limit` header into the outbound request
- **AND** the host returns the downstream response status and body to the original client

## ADDED Requirements

### Requirement: Host enforces a request hop limit for inbound and outbound mesh traffic
The `core-host` gateway SHALL track a request-scoped hop limit using the `X-Tachyon-Hop-Limit` header so distributed routing loops are rejected before they can exhaust host resources.

#### Scenario: Client omits the hop-limit header
- **WHEN** a client sends a request without `X-Tachyon-Hop-Limit`
- **THEN** the host assigns the request a default hop limit of `10`
- **AND** the request continues through normal route resolution

#### Scenario: A loop exhausts the remaining hops
- **WHEN** an inbound request arrives with `X-Tachyon-Hop-Limit: 0`
- **THEN** the host rejects the request before guest execution starts
- **AND** the HTTP response status is `508 Loop Detected`
- **AND** the response body explains that the routing loop exceeded the hop limit

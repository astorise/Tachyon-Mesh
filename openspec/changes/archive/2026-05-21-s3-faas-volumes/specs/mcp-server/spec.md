## ADDED Requirements

### Requirement: MCP exposes tools to list, attach, and detach S3 volumes on routes
The Tachyon MCP server SHALL provide three tools for managing S3 volumes on FaaS routes, operating on the live sealed manifest via the admin API.

#### Scenario: list_s3_volumes returns S3 volumes for a route
- **WHEN** an AI agent calls `list_s3_volumes` with a `route_path` argument
- **THEN** the tool returns a list of S3 volume configurations (bucket, prefix, guest_path, readonly) for that route
- **AND** returns an empty list if the route has no S3 volumes

#### Scenario: attach_s3_volume adds an S3 volume to a route
- **WHEN** an AI agent calls `attach_s3_volume` with `route_path`, `s3_url`, `guest_path`, and `readonly`
- **THEN** the tool adds the S3 volume to the route's configuration in the sealed manifest
- **AND** returns the updated route configuration
- **AND** subsequent invocations of the route receive the S3 volume

#### Scenario: detach_s3_volume removes an S3 volume from a route
- **WHEN** an AI agent calls `detach_s3_volume` with `route_path` and `guest_path`
- **THEN** the tool removes the matching S3 volume from the route's configuration
- **AND** subsequent invocations no longer receive that volume

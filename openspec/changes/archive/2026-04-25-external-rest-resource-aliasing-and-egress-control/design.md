# Design: Sealed Resource Aliases and User Egress Control

## Runtime shape
The current workspace already has one outbound resolution path in `core-host`:

- `MESH_FETCH:` guest responses
- the `tachyon::mesh::outbound_http` host bindings used by system FaaS
- SemVer-aware internal dependency routing through `RouteRegistry`

Instead of introducing a second resolver layer, this change extends that existing path with a
sealed top-level `resources` map in `integrity.lock`.

## Manifest schema
The signed payload may now declare logical resource aliases:

```json
{
  "resources": {
    "inventory-api": {
      "type": "internal",
      "target": "inventory",
      "version_constraint": "^1.2.0"
    },
    "payment-gateway": {
      "type": "external",
      "target": "https://api.stripe.com/v1",
      "allowed_methods": ["POST"]
    }
  }
}
```

Notes:

- `internal.target` may reference either a sealed route path or a sealed logical route name.
- `internal.version_constraint` is optional and is matched against the sealed route version.
- `external.target` must be HTTPS in production. Plain HTTP is accepted only for localhost-style
  test targets.
- `external.allowed_methods` is required and normalized to uppercase.
- Resource names must not collide with sealed route names.

## Resolution model
Guests continue to call logical URLs such as `http://mesh/payment-gateway/charges`.

Resolution order:

1. Direct sealed route path
2. Sealed resource alias
3. Existing SemVer dependency name resolution

For internal aliases, the host rewrites to the local mesh base URL and preserves the suffix path
and query string.

For external aliases, the host rewrites to the configured external base URL, preserves the suffix
path and query string, and enforces the sealed HTTP method allow-list before the request is sent.

## Security model
This change intentionally distinguishes user routes from privileged system routes:

- `user` routes may call internal mesh targets directly and may call external services only through
  a sealed external resource alias.
- `system` routes keep the existing raw outbound capability for infrastructure integrations already
  present in the repo, such as Kubernetes, S3, SQS, and gossip peers.

For external targets, the host strips Tachyon-specific and hop-by-hop headers before forwarding the
request so identity material and mesh routing metadata do not leak out of the cluster.

## Manifest production
The repo no longer contains a standalone `tachyon-cli` manifest generator. The implementation path
for this change is therefore:

- the checked-in `integrity.lock`
- `core-host` config validation
- `core-host` test helpers that synthesize sealed manifests during unit/integration tests

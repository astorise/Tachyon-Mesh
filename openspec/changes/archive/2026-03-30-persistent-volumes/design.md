# Design: Change 022 - Persistent Volumes

## Summary
Route-scoped volume mounts are sealed into `integrity.lock` and injected into the
request-scoped WASI context before a guest runs. The same sealed shape is used by
`tachyon-cli` and `core-host`, which keeps signing and validation deterministic.

## CLI Shape
`tachyon-cli generate` accepts repeatable `--volume` arguments.

- Explicit route binding: `/api/guest-volume=/tmp/tachyon_data:/app/data:rw`
- Implicit binding is allowed only when exactly one sealed route exists:
  `/tmp/tachyon_data:/app/data:rw`
- `:ro` makes the mount read-only; the mode defaults to `rw`

The parser splits the mapping from the right so Linux paths and Windows drive
letters can coexist with a POSIX guest path.

## Sealed Manifest
Each sealed route may contain:

```json
{
  "path": "/api/guest-volume",
  "role": "user",
  "volumes": [
    {
      "host_path": "/tmp/tachyon_data",
      "guest_path": "/app/data",
      "readonly": false
    }
  ]
}
```

Older manifests remain valid because `volumes` defaults to an empty list.

## Host Wiring
`core-host` validates each sealed volume, normalizes guest paths, rejects
duplicate guest mount points per route, and preopens every configured directory
into the route's WASI context.

- Legacy WASI guests keep the existing `.` preopen for their artifact directory.
- Component guests receive the same route volumes through their preview2
  `WasiCtxBuilder`.
- Read-only mounts expose `DirPerms::READ` and `FilePerms::READ`.
- Read-write mounts add `DirPerms::MUTATE` and `FilePerms::WRITE`.

## Validation Guest
`guest-volume` is a component guest that writes `POST` bodies to
`/app/data/state.txt` and returns the stored value on `GET`, which proves that
the preopened mount is shared with the host filesystem.

# Tachyon Mesh Go SDK

Official Go SDK bindings for Tachyon Mesh FaaS guests, covering the
`kv-partition` interface (the `kv-consumer` world in `wit/tachyon.wit`).

```bash
go get github.com/astorise/Tachyon-Mesh/sdk/go@v1.0.0
```

## Quick Start

```go
package main

import (
	tachyonsdk "github.com/astorise/Tachyon-Mesh/sdk/go"
)

func main() {
	table := tachyonsdk.NewTable("viking-context")
	defer table.Close()

	if err := table.BatchSet(map[string][]byte{
		"node:1": []byte("data1"),
		"node:2": []byte("data2"),
	}); err != nil {
		panic(err)
	}

	history, err := table.GetRange("timestamp:20260512T0000", "timestamp:20260512T2359", 50, 0)
	if err != nil {
		panic(err)
	}
	_ = history
}
```

Build a real guest component with [TinyGo](https://tinygo.org/) (mainline
Go's `gc` compiler does not support the Component Model ABI that the
generated bindings in `gen/` use -- `go build`/`go vet` work for local
development and type-checking, but producing an actual `.wasm` guest
requires TinyGo's `wasip2` target):

```bash
tinygo build -o guest.wasm --target=wasip2 .
```

## Regenerating bindings

`gen/` is real, generated Component Model bindings (not hand-written stubs),
produced by [`wit-bindgen-go`](https://github.com/bytecodealliance/wit-bindgen-go)
from `wit/tachyon.wit` -- a vendored, byte-for-byte copy of
`sdk/wit/tachyon.wit` kept in sync by `scripts/check_wit_sdk_sync.sh`.
Regenerate after any `kv-partition` WIT change with:

```bash
go generate ./...
```

Release automation (`publish-sdks.yml`) re-runs the same generator and
diffs the result against the committed `gen/` tree before publishing, so a
stale, un-regenerated `gen/` tree fails CI rather than shipping silently
out of date.

## Versioning

This module lives in a subdirectory of the `astorise/Tachyon-Mesh` monorepo,
so Go's module proxy resolves versions from `sdk/go/vX.Y.Z` tags (not the
repo-root `vX.Y.Z` release tag) -- `publish-sdks.yml` pushes that tag
alongside the root release tag from the exact same commit.

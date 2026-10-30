package tachyonsdk

import (
	"errors"

	kvpartition "github.com/astorise/Tachyon-Mesh/sdk/go/gen/tachyon/mesh/kv-partition"
	"go.bytecodealliance.org/cm"
)

// Table is a handle to a named key-value partition backed by redb on the
// host. Unlike the Rust SDK's Table, Go has no resource destructor the
// Wasm runtime can call automatically on scope exit -- call Close (or
// defer it) once a Table is no longer needed to release the host-side
// tracking entry.
type Table struct {
	inner kvpartition.Table
}

// NewTable opens or creates the named KV partition on the host.
func NewTable(name string) *Table {
	return &Table{inner: kvpartition.NewTable(name)}
}

// Close releases the host-side tracking entry for this Table.
func (t *Table) Close() {
	t.inner.ResourceDrop()
}

// Get returns the value stored under key.
func (t *Table) Get(key string) ([]byte, error) {
	result := t.inner.Get(key)
	ok, errMsg, isErr := result.Result()
	if isErr {
		return nil, errors.New(errMsg)
	}
	return ok.Slice(), nil
}

// Set stores value under key.
func (t *Table) Set(key string, value []byte) error {
	result := t.inner.Set(key, cm.ToList(value))
	_, errMsg, isErr := result.Result()
	if isErr {
		return errors.New(errMsg)
	}
	return nil
}

// CompareAndSet atomically stores value under key only if the key's
// current value matches expected (nil expected means "key must not
// exist yet"). It reports whether the swap took place.
func (t *Table) CompareAndSet(key string, expected []byte, value []byte) (bool, error) {
	var expectedOpt cm.Option[cm.List[byte]]
	if expected != nil {
		expectedOpt = cm.Some(cm.ToList(expected))
	}
	result := t.inner.CompareAndSet(key, expectedOpt, cm.ToList(value))
	swapped, errMsg, isErr := result.Result()
	if isErr {
		return false, errors.New(errMsg)
	}
	return swapped, nil
}

// CompareAndDelete atomically deletes key only if its current value
// matches expected. It reports whether the delete took place.
func (t *Table) CompareAndDelete(key string, expected []byte) (bool, error) {
	result := t.inner.CompareAndDelete(key, cm.ToList(expected))
	deleted, errMsg, isErr := result.Result()
	if isErr {
		return false, errors.New(errMsg)
	}
	return deleted, nil
}

// Delete removes key.
func (t *Table) Delete(key string) error {
	result := t.inner.Delete(key)
	_, errMsg, isErr := result.Result()
	if isErr {
		return errors.New(errMsg)
	}
	return nil
}

// BatchSet stores multiple key/value entries in a single host call.
func (t *Table) BatchSet(entries map[string][]byte) error {
	tuples := make([]cm.Tuple[string, cm.List[byte]], 0, len(entries))
	for k, v := range entries {
		tuples = append(tuples, cm.Tuple[string, cm.List[byte]]{F0: k, F1: cm.ToList(v)})
	}
	result := t.inner.BatchSet(cm.ToList(tuples))
	_, errMsg, isErr := result.Result()
	if isErr {
		return errors.New(errMsg)
	}
	return nil
}

// GetRange returns up to limit key/value entries with keys in
// [startKey, endKey), skipping the first offset matches.
func (t *Table) GetRange(startKey, endKey string, limit, offset uint32) (map[string][]byte, error) {
	result := t.inner.GetRange(startKey, endKey, limit, offset)
	ok, errMsg, isErr := result.Result()
	if isErr {
		return nil, errors.New(errMsg)
	}
	entries := ok.Slice()
	out := make(map[string][]byte, len(entries))
	for _, entry := range entries {
		out[entry.F0] = entry.F1.Slice()
	}
	return out, nil
}

# Specifications: Embedded Core Store

## 1. Storage Configuration
The host needs a single directory path to store its database file. If Tachyon runs inside a Docker container, this path MUST be mounted as a persistent volume.

    {
        "host": {
            "data_dir": "/var/lib/tachyon/data"
        }
    }
    // The core will create `/var/lib/tachyon/data/tachyon.db`

## 2. Table Definitions (B-Tree Buckets)
`redb` uses strictly typed tables. The `CoreStore` will define three main tables:

* **Table `cwasm_cache`**: 
    * *Key:* `&str` (SHA-256 Hash of the original `.wasm` file + Wasmtime compiler version).
    * *Value:* `&[u8]` (Serialized machine code).
* **Table `tls_certs`**: 
    * *Key:* `&str` (Domain name, e.g., "api.gutlab.com").
    * *Value:* `&[u8]` (JSON containing `fullchain.pem`, `privkey.pem`, and expiration timestamp).
* **Table `hibernation_state`**:
    * *Key:* `&str` (Target FaaS Name / Instance ID).
    * *Value:* `&[u8]` (Zipped memory snapshot).

## 3. Concurrency Model (Single-Writer / Multi-Reader)
- `redb` allows parallel read transactions (ultra-fast via memory-mapping). The HTTP router can query the `tls_certs` table for every incoming connection without blocking.
- `redb` only allows **one active write transaction at a time**. Write requests (e.g., saving a new cert or hibernating a RAM volume) will safely queue up internally.
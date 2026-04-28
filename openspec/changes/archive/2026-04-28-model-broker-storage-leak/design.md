# Design: Atomic Streaming Architecture

## 1. Stream Processing (`systems/system-faas-model-broker/src/lib.rs`)
The streaming logic handling the incoming data chunks must be updated.

### File Creation
Instead of creating the target file immediately:
```rust
let final_path = PathBuf::from(format!("{}/{}", model_dir, request.filename));
let part_path = final_path.with_extension("part");

let mut file = fs::File::create(&part_path)?;
```

### Stream Loop & Atomic Rename
The loop writes chunks to `file`. Once the stream yields `None` (completion):
```rust
// Flush the buffer to disk
file.sync_all()?;
drop(file); // Release file lock

// Atomic rename
fs::rename(&part_path, &final_path)?;
```

## 2. Error Handling & Cleanup
If the network stream yields an error, or if the client disconnects prematurely:
- The function must intercept the error.
- It must immediately call `fs::remove_file(&part_path)`.
- It must wrap the removal in a `match` to avoid panicking if the file is already gone or locked (graceful degradation).

## 3. Garbage Collection Integration
No code changes are required in `system-faas-gc`. However, administrators should be documented to configure a `TTL_SECONDS` (e.g., `7200` for 2 hours) on the `model-broker` volume mount in `integrity.lock` to ensure any `.part` files that escape the immediate cleanup are eventually purged.
# Implementation Tasks

## Phase 1: Temporary File Logic
- [ ] Open `systems/system-faas-model-broker/src/lib.rs`.
- [ ] Locate the logic responsible for creating and writing the incoming model stream to disk.
- [ ] Modify the file creation logic to append `.part` to the requested filename.

## Phase 2: Atomic Completion
- [ ] Ensure `sync_all()` is called on the file descriptor when the stream finishes to guarantee data is flushed to the physical disk.
- [ ] Implement the `fs::rename` call to atomically move the `.part` file to its final `.gguf` (or standard) extension.

## Phase 3: Immediate Failure Cleanup
- [ ] Wrap the streaming loop in error handling.
- [ ] If an error is detected (e.g., network timeout, unexpected EOF), attempt to delete the `.part` file immediately using a safe `match fs::remove_file(...)` pattern.

## Phase 4: Validation
- [ ] **Test Abort:** Start a large model upload to the broker and kill the client process halfway. Verify that the `.part` file is deleted (or remains as `.part` if hard-killed) and NO corrupted final file exists.
- [ ] **Test Success:** Complete a full upload and verify the `.part` extension is removed.
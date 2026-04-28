# Design: Resilient Sweeper Architecture

## 1. Error Handling (`systems/system-faas-gc/src/main.rs`)
The `sweep_directory` function must be updated to replace the hard-fail `?` operators on deletion actions.

### Graceful File Deletion
```rust
match fs::remove_file(&entry_path) {
    Ok(_) => {
        println!("deleted {}", entry_path.display());
        removed_files += 1;
    }
    Err(e) => {
        println!("failed to delete {} (ignoring): {}", entry_path.display(), e);
    }
}
```

## 2. Directory Reaping
The recursive descent logic must evaluate directory states upon returning from the recursion. 

### Workflow:
1. Iterate through a directory.
2. If an entry is a directory, call `sweep_directory` recursively.
3. Once the loop finishes, check if the current directory is now empty.
4. If empty (and it is not the root `TARGET_DIR`), delete it.
5. Catch and ignore `DirectoryNotEmpty` or `PermissionDenied` errors during directory removal, as other concurrent FaaS instances might have written a new file into it right after our check.
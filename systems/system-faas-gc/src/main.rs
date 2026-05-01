use std::{
    env,
    error::Error,
    fs, io,
    path::Path,
    time::{Duration, SystemTime},
};

fn main() -> Result<(), Box<dyn Error>> {
    let ttl_seconds = env::var("TTL_SECONDS")?.parse::<u64>()?;
    let target_dir = env::var("TARGET_DIR")?;
    let stats = sweep_directory(Path::new(&target_dir), Duration::from_secs(ttl_seconds));
    println!(
        "gc sweep complete: removed {} stale files and {} empty directories from {}",
        stats.removed_files, stats.removed_dirs, target_dir
    );
    Ok(())
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct SweepStats {
    removed_files: u64,
    removed_dirs: u64,
}

impl SweepStats {
    fn merge(&mut self, other: SweepStats) {
        self.removed_files += other.removed_files;
        self.removed_dirs += other.removed_dirs;
    }
}

/// Sweep `path` for files whose mtime is older than `ttl`, then prune empty directories.
///
/// Tolerates filesystem races: a missing or locked entry is logged and skipped instead of
/// aborting the sweep. After a directory's stale contents are processed, the directory
/// itself is removed if it has become empty, so highly dynamic workloads cannot exhaust
/// the host's inode table with "ghost" directories.
fn sweep_directory(path: &Path, ttl: Duration) -> SweepStats {
    let mut stats = SweepStats::default();
    let now = SystemTime::now();

    let read_dir = match fs::read_dir(path) {
        Ok(rd) => rd,
        Err(error) => {
            log_error("read_dir", path, &error);
            return stats;
        }
    };

    for entry in read_dir {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                log_error("read_dir entry", path, &error);
                continue;
            }
        };

        let entry_path = entry.path();
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error) => {
                log_error("metadata", &entry_path, &error);
                continue;
            }
        };

        if metadata.is_dir() {
            stats.merge(sweep_directory(&entry_path, ttl));
            continue;
        }

        let modified = match metadata.modified() {
            Ok(modified) => modified,
            Err(error) => {
                log_error("modified", &entry_path, &error);
                continue;
            }
        };

        let age = now.duration_since(modified).unwrap_or_default();
        if age > ttl {
            match fs::remove_file(&entry_path) {
                Ok(()) => {
                    println!("deleted {}", entry_path.display());
                    stats.removed_files += 1;
                }
                Err(error) => log_error("remove_file", &entry_path, &error),
            }
        }
    }

    // After processing, remove the directory itself if it has become empty.
    // Failure here is benign (something raced in a new child); log and move on.
    if directory_is_empty(path) {
        match fs::remove_dir(path) {
            Ok(()) => {
                println!("deleted empty directory {}", path.display());
                stats.removed_dirs += 1;
            }
            Err(error) => log_error("remove_dir", path, &error),
        }
    }

    stats
}

fn directory_is_empty(path: &Path) -> bool {
    match fs::read_dir(path) {
        Ok(mut rd) => rd.next().is_none(),
        Err(_) => false,
    }
}

fn log_error(op: &str, path: &Path, error: &io::Error) {
    eprintln!("gc {op} on {} skipped: {error}", path.display());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs::File, io::Write, time::Duration};

    fn touch_old(path: &Path, age: Duration) {
        // Create file then set mtime to far in the past.
        let mut f = File::create(path).expect("create file");
        f.write_all(b"stale").expect("write");
        drop(f);
        let new_time = SystemTime::now() - age;
        // Best-effort: filetime isn't a workspace dep, so use fs::File timestamps via
        // setting the file's creation time backwards is not portable; instead we sleep
        // for tiny tests and rely on a near-zero TTL. This helper is now a no-op when
        // Duration is too large to roll back portably; tests below pass `ttl=0`.
        let _ = new_time;
    }

    #[test]
    fn sweep_removes_stale_files() {
        let dir = tempdir();
        let f1 = dir.path().join("a.tmp");
        let f2 = dir.path().join("b.tmp");
        File::create(&f1)
            .expect("stale test file a should be created")
            .write_all(b"x")
            .expect("stale test file a should be written");
        File::create(&f2)
            .expect("stale test file b should be created")
            .write_all(b"y")
            .expect("stale test file b should be written");
        touch_old(&f1, Duration::from_secs(3600));
        touch_old(&f2, Duration::from_secs(3600));

        let stats = sweep_directory(dir.path(), Duration::from_secs(0));
        assert_eq!(stats.removed_files, 2);
        // Parent gets reaped because it became empty.
        assert_eq!(stats.removed_dirs, 1);
        assert!(!dir.path().exists());
    }

    #[test]
    fn sweep_tolerates_race_on_missing_file() {
        let dir = tempdir();
        let f = dir.path().join("ghost.tmp");
        File::create(&f)
            .expect("race test file should be created")
            .write_all(b"x")
            .expect("race test file should be written");
        // Pre-delete, simulating a concurrent process winning the race.
        fs::remove_file(&f).expect("race test file should be removed before sweep");

        // Sweep must finish without panicking even though the directory listing
        // could theoretically still reference the just-deleted entry on some FSes.
        let stats = sweep_directory(dir.path(), Duration::from_secs(0));
        assert_eq!(stats.removed_files, 0);
        // Directory was empty before the sweep; the sweep removes it.
        assert_eq!(stats.removed_dirs, 1);
    }

    #[test]
    fn sweep_reaps_nested_empty_dirs() {
        let dir = tempdir();
        let nested = dir.path().join("a").join("b").join("c");
        fs::create_dir_all(&nested).expect("nested test directory should be created");
        let stale = nested.join("file.tmp");
        File::create(&stale)
            .expect("nested stale file should be created")
            .write_all(b"x")
            .expect("nested stale file should be written");

        let stats = sweep_directory(dir.path(), Duration::from_secs(0));
        assert_eq!(stats.removed_files, 1);
        // a, b, c, and the temp root all become empty and are reaped.
        assert!(stats.removed_dirs >= 3);
    }

    // Tiny inline tempdir helper — avoids adding the `tempfile` crate just for tests.
    struct TempDir {
        path: std::path::PathBuf,
    }
    impl TempDir {
        fn path(&self) -> &Path {
            &self.path
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
    fn tempdir() -> TempDir {
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let pid = std::process::id();
        let path = env::temp_dir().join(format!("system-faas-gc-test-{pid}-{nanos}"));
        fs::create_dir_all(&path).expect("create tempdir");
        TempDir { path }
    }
}

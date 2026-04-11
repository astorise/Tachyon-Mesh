use std::{
    env,
    error::Error,
    fs,
    path::Path,
    time::{Duration, SystemTime},
};

fn main() -> Result<(), Box<dyn Error>> {
    let ttl_seconds = env::var("TTL_SECONDS")?.parse::<u64>()?;
    let target_dir = env::var("TARGET_DIR")?;
    let removed_files = sweep_directory(Path::new(&target_dir), Duration::from_secs(ttl_seconds))?;
    println!(
        "gc sweep complete: removed {removed_files} stale files from {}",
        target_dir
    );
    Ok(())
}

fn sweep_directory(path: &Path, ttl: Duration) -> Result<u64, Box<dyn Error>> {
    let mut removed_files = 0_u64;
    let now = SystemTime::now();

    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let entry_path = entry.path();
        let metadata = entry.metadata()?;

        if metadata.is_dir() {
            removed_files += sweep_directory(&entry_path, ttl)?;
            continue;
        }

        let modified = metadata.modified()?;
        let age = now.duration_since(modified).unwrap_or_default();
        if age > ttl {
            fs::remove_file(&entry_path)?;
            println!("deleted {}", entry_path.display());
            removed_files += 1;
        }
    }

    Ok(removed_files)
}

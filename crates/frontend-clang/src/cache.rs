//! Per-TU graph cache backed by on-disk JSON files.
//!
//! Cache key: seahash of (source file bytes ++ compiler args ++ compile_commands.json mtime).
//! The mtime of compile_commands.json acts as a coarse invalidation signal for
//! header changes — when you re-run CMake the mtime bumps and the cache misses.
//!
//! Cache lives in `.spaghetti-cache/` next to `compile_commands.json`.

use std::path::{Path, PathBuf};

use core_ir::Graph;
use tracing::{debug, warn};

/// Location of the cache directory, derived from compile_commands.json's parent.
pub(crate) fn cache_dir(compile_commands: &Path) -> PathBuf {
    compile_commands
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".spaghetti-cache")
}

/// Compute a cache key for a translation unit.
///
/// Hashes: source file contents + compiler args + compile_commands.json mtime
/// (as a coarse proxy for header changes).
pub(crate) fn cache_key(source_path: &Path, args: &[String], cc_mtime: u64) -> u64 {
    use seahash::SeaHasher;
    use std::hash::{Hash, Hasher};

    let mut h = SeaHasher::new();

    // Hash source file contents (the main signal for changes).
    if let Ok(contents) = std::fs::read(source_path) {
        contents.hash(&mut h);
    } else {
        // Can't read the file — hash the path so we still get a unique key.
        source_path.hash(&mut h);
    }

    // Hash compiler args (flags like -D, -std change semantics).
    args.hash(&mut h);

    // Hash compile_commands.json mtime as a coarse invalidator for header changes.
    cc_mtime.hash(&mut h);

    h.finish()
}

/// Try to load a cached graph for the given key.
pub(crate) fn load(cache_dir: &Path, key: u64) -> Option<Graph> {
    let path = cache_dir.join(format!("{key:016x}.json"));
    let bytes = std::fs::read(&path).ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(graph) => {
            debug!(key = %format!("{key:016x}"), "cache hit");
            Some(graph)
        }
        Err(e) => {
            warn!(key = %format!("{key:016x}"), error = %e, "corrupt cache entry, ignoring");
            let _ = std::fs::remove_file(&path);
            None
        }
    }
}

/// Store a graph in the cache.
pub(crate) fn store(cache_dir: &Path, key: u64, graph: &Graph) {
    if let Err(e) = std::fs::create_dir_all(cache_dir) {
        warn!(error = %e, "failed to create cache dir");
        return;
    }
    let path = cache_dir.join(format!("{key:016x}.json"));
    match serde_json::to_vec(graph) {
        Ok(bytes) => {
            if let Err(e) = std::fs::write(&path, bytes) {
                warn!(error = %e, "failed to write cache entry");
            }
        }
        Err(e) => {
            warn!(error = %e, "failed to serialize cache entry");
        }
    }
}

/// Get compile_commands.json mtime as seconds since epoch (or 0 on error).
pub(crate) fn file_mtime_secs(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

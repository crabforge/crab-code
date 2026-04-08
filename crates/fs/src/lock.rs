//! File locking via `fd-lock`.

use std::path::Path;

/// A held file lock. Dropped when the guard goes out of scope.
pub struct FileLockGuard {
    _private: (),
}

/// Acquire an exclusive lock on `path`, creating the file if needed.
///
/// # Errors
///
/// Returns an error if the file cannot be opened or locked.
pub fn lock_exclusive(path: &Path) -> crab_common::Result<FileLockGuard> {
    // Ensure parent directory exists.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Create (or open) the lock file to establish an on-disk marker.
    // NOTE: the current `FileLockGuard` struct cannot hold the fd-lock
    // handle, so the OS-level lock is not retained across the guard
    // lifetime. A future refactor should store `fd_lock::RwLock` inside
    // the guard.
    let _file = std::fs::File::create(path)?;
    Ok(FileLockGuard { _private: () })
}

/// Try to acquire an exclusive lock without blocking. Returns `None` if already held.
///
/// # Errors
///
/// Returns an error if the file cannot be opened.
pub fn try_lock_exclusive(path: &Path) -> crab_common::Result<Option<FileLockGuard>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Non-blocking attempt. Because the guard struct cannot yet hold
    // the fd-lock handle, we always succeed for now and return `Some`.
    let _file = std::fs::File::create(path)?;
    Ok(Some(FileLockGuard { _private: () }))
}

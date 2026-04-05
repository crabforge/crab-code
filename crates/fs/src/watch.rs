//! File system watching via [`notify`].
//!
//! Provides a [`FileWatcher`] that monitors a directory tree and delivers
//! [`WatchEvent`]s through a channel. Useful for hot-reloading configuration
//! files (`CRAB.md`, `settings.json`) and detecting workspace changes.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

// ── Public types ──────────────────────────────────────────────────────

/// Simplified events emitted by the file watcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    /// A file was created.
    Created(PathBuf),
    /// A file was modified (content or metadata).
    Modified(PathBuf),
    /// A file was removed.
    Removed(PathBuf),
    /// A file was renamed (from, to).
    Renamed { from: PathBuf, to: PathBuf },
}

/// Watches a directory tree for file changes using the OS-native backend
/// (inotify on Linux, `ReadDirectoryChanges` on Windows, `FSEvents` on macOS).
pub struct FileWatcher {
    /// The underlying notify watcher. Kept alive to maintain the OS watch.
    _watcher: RecommendedWatcher,
    /// Channel receiving simplified events.
    receiver: mpsc::Receiver<WatchEvent>,
    /// Root path being watched.
    root: PathBuf,
}

impl FileWatcher {
    /// Start watching `path` recursively for changes.
    ///
    /// Events are buffered in an internal channel; use [`poll`] or
    /// [`poll_timeout`] to drain them.
    ///
    /// # Errors
    ///
    /// Returns an error if the path does not exist or cannot be watched.
    pub fn new(path: &Path) -> crab_common::Result<Self> {
        let (tx, rx) = mpsc::channel();

        let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                for we in translate_event(&event) {
                    let _ = tx.send(we);
                }
            }
        })
        .map_err(|e| crab_common::Error::Other(format!("failed to create watcher: {e}")))?;

        watcher.watch(path, RecursiveMode::Recursive).map_err(|e| {
            crab_common::Error::Other(format!("failed to watch {}: {e}", path.display()))
        })?;

        Ok(Self {
            _watcher: watcher,
            receiver: rx,
            root: path.to_path_buf(),
        })
    }

    /// The root directory being watched.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Drain all currently buffered events (non-blocking).
    #[must_use]
    pub fn poll(&self) -> Vec<WatchEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.receiver.try_recv() {
            events.push(event);
        }
        events
    }

    /// Wait for an event with a timeout.
    ///
    /// Returns `None` if no event arrives within `timeout`.
    #[must_use]
    pub fn poll_timeout(&self, timeout: Duration) -> Option<WatchEvent> {
        self.receiver.recv_timeout(timeout).ok()
    }

    /// Stop watching. After this call, no more events will be received.
    /// The watcher is also stopped automatically on drop.
    pub fn stop(self) {
        // Dropping `self._watcher` stops the OS-level watch.
        drop(self);
    }
}

// ── Event translation ─────────────────────────────────────────────────

/// Translate a `notify::Event` into zero or more `WatchEvent`s.
fn translate_event(event: &Event) -> Vec<WatchEvent> {
    let mut result = Vec::new();

    match &event.kind {
        EventKind::Create(_) => {
            for path in &event.paths {
                result.push(WatchEvent::Created(path.clone()));
            }
        }
        EventKind::Modify(_) => {
            for path in &event.paths {
                result.push(WatchEvent::Modified(path.clone()));
            }
        }
        EventKind::Remove(_) => {
            for path in &event.paths {
                result.push(WatchEvent::Removed(path.clone()));
            }
        }
        _ => {
            // Access, Other, etc. — ignored
        }
    }

    result
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn translate_create_event() {
        let event = Event {
            kind: EventKind::Create(notify::event::CreateKind::File),
            paths: vec![PathBuf::from("/tmp/test.txt")],
            attrs: notify::event::EventAttributes::default(),
        };
        let events = translate_event(&event);
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            WatchEvent::Created(PathBuf::from("/tmp/test.txt"))
        );
    }

    #[test]
    fn translate_modify_event() {
        let event = Event {
            kind: EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Content,
            )),
            paths: vec![PathBuf::from("/tmp/test.txt")],
            attrs: notify::event::EventAttributes::default(),
        };
        let events = translate_event(&event);
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            WatchEvent::Modified(PathBuf::from("/tmp/test.txt"))
        );
    }

    #[test]
    fn translate_remove_event() {
        let event = Event {
            kind: EventKind::Remove(notify::event::RemoveKind::File),
            paths: vec![PathBuf::from("/tmp/test.txt")],
            attrs: notify::event::EventAttributes::default(),
        };
        let events = translate_event(&event);
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            WatchEvent::Removed(PathBuf::from("/tmp/test.txt"))
        );
    }

    #[test]
    fn translate_other_event_ignored() {
        let event = Event {
            kind: EventKind::Other,
            paths: vec![PathBuf::from("/tmp/test.txt")],
            attrs: notify::event::EventAttributes::default(),
        };
        let events = translate_event(&event);
        assert!(events.is_empty());
    }

    #[test]
    fn translate_multi_path_event() {
        let event = Event {
            kind: EventKind::Create(notify::event::CreateKind::File),
            paths: vec![PathBuf::from("/tmp/a.txt"), PathBuf::from("/tmp/b.txt")],
            attrs: notify::event::EventAttributes::default(),
        };
        let events = translate_event(&event);
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn watcher_creates_and_detects_file() {
        let dir = tempfile::tempdir().unwrap();
        let watcher = FileWatcher::new(dir.path()).unwrap();
        assert_eq!(watcher.root(), dir.path());

        // Create a file — should trigger an event
        let file_path = dir.path().join("new_file.txt");
        fs::write(&file_path, "hello").unwrap();

        // Give the OS a moment to deliver the event
        std::thread::sleep(Duration::from_millis(200));

        let events = watcher.poll();
        // We should have at least one Created or Modified event
        assert!(
            !events.is_empty(),
            "Expected at least one event after file creation"
        );
    }

    #[test]
    fn watcher_detects_modification() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        fs::write(&file_path, "initial").unwrap();

        // Small delay to avoid conflating create with modify
        std::thread::sleep(Duration::from_millis(100));

        let watcher = FileWatcher::new(dir.path()).unwrap();

        // Modify the file
        fs::write(&file_path, "modified").unwrap();
        std::thread::sleep(Duration::from_millis(200));

        let events = watcher.poll();
        assert!(!events.is_empty());
    }

    #[test]
    fn watcher_detects_removal() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("to_delete.txt");
        fs::write(&file_path, "bye").unwrap();
        std::thread::sleep(Duration::from_millis(100));

        let watcher = FileWatcher::new(dir.path()).unwrap();

        fs::remove_file(&file_path).unwrap();
        std::thread::sleep(Duration::from_millis(200));

        let events = watcher.poll();
        assert!(!events.is_empty());
    }

    #[test]
    fn poll_timeout_returns_none_when_no_events() {
        let dir = tempfile::tempdir().unwrap();
        let watcher = FileWatcher::new(dir.path()).unwrap();
        let result = watcher.poll_timeout(Duration::from_millis(50));
        assert!(result.is_none());
    }

    #[test]
    fn watcher_nonexistent_path_errors() {
        let result = FileWatcher::new(Path::new("/nonexistent/path/that/does/not/exist"));
        assert!(result.is_err());
    }

    #[test]
    fn watch_event_equality() {
        let a = WatchEvent::Created(PathBuf::from("/a"));
        let b = WatchEvent::Created(PathBuf::from("/a"));
        assert_eq!(a, b);

        let c = WatchEvent::Modified(PathBuf::from("/a"));
        assert_ne!(a, c);
    }

    #[test]
    fn watch_event_renamed_variant() {
        let e = WatchEvent::Renamed {
            from: PathBuf::from("/a"),
            to: PathBuf::from("/b"),
        };
        assert!(matches!(e, WatchEvent::Renamed { .. }));
    }
}

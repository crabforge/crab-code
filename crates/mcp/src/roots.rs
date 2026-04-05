//! MCP roots support — `roots/list` and `roots/list_changed`.
//!
//! Allows clients to inform servers about the working directories (roots)
//! they have access to, and to notify servers when roots change.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Information about a root directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootInfo {
    /// URI of the root (e.g. `file:///home/user/project`).
    pub uri: String,
    /// Human-readable name for this root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl RootInfo {
    /// Create a root from a URI string.
    #[must_use]
    pub fn new(uri: impl Into<String>) -> Self {
        Self {
            uri: uri.into(),
            name: None,
        }
    }

    /// Create a root with a name.
    #[must_use]
    pub fn with_name(uri: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            uri: uri.into(),
            name: Some(name.into()),
        }
    }

    /// Create a root from a filesystem path, converting to a `file://` URI.
    #[must_use]
    pub fn from_path(path: &Path) -> Self {
        let uri = path_to_file_uri(path);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned());
        Self { uri, name }
    }

    /// Try to extract the filesystem path from the URI.
    #[must_use]
    pub fn to_path(&self) -> Option<PathBuf> {
        file_uri_to_path(&self.uri)
    }
}

/// Registry of roots that the client exposes to servers.
#[derive(Debug, Default)]
pub struct RootRegistry {
    roots: Vec<RootInfo>,
    version: u64,
}

impl RootRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a root. Returns true if the root was added (not a duplicate URI).
    pub fn add(&mut self, root: RootInfo) -> bool {
        if self.roots.iter().any(|r| r.uri == root.uri) {
            return false;
        }
        self.roots.push(root);
        self.version += 1;
        true
    }

    /// Remove a root by URI. Returns true if found and removed.
    pub fn remove(&mut self, uri: &str) -> bool {
        let before = self.roots.len();
        self.roots.retain(|r| r.uri != uri);
        if self.roots.len() < before {
            self.version += 1;
            true
        } else {
            false
        }
    }

    /// Replace all roots at once.
    pub fn set_roots(&mut self, roots: Vec<RootInfo>) {
        self.roots = roots;
        self.version += 1;
    }

    /// List current roots.
    #[must_use]
    pub fn list(&self) -> &[RootInfo] {
        &self.roots
    }

    /// Number of roots.
    #[must_use]
    pub fn len(&self) -> usize {
        self.roots.len()
    }

    /// Whether the registry is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    /// Monotonically increasing version, bumped on every change.
    /// Can be used to detect whether a `roots/list_changed` notification
    /// needs to be sent.
    #[must_use]
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Check if a given path falls under any registered root.
    #[must_use]
    pub fn contains_path(&self, path: &Path) -> bool {
        self.roots.iter().any(|r| {
            r.to_path()
                .is_some_and(|root_path| path.starts_with(&root_path))
        })
    }
}

/// Convert a filesystem path to a `file://` URI.
#[must_use]
pub fn path_to_file_uri(path: &Path) -> String {
    let abs = if path.is_absolute() {
        path.to_string_lossy().into_owned()
    } else {
        format!("/{}", path.to_string_lossy())
    };
    // Normalize backslashes to forward slashes for URI
    let normalized = abs.replace('\\', "/");
    // Ensure leading slash for Windows paths like C:\...
    if normalized.starts_with('/') {
        format!("file://{normalized}")
    } else {
        format!("file:///{normalized}")
    }
}

/// Convert a `file://` URI back to a filesystem path.
#[must_use]
pub fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    let stripped = uri.strip_prefix("file://")?;
    // On Windows, file:///C:/... → C:/...
    #[cfg(target_os = "windows")]
    {
        let path_str = stripped.strip_prefix('/').unwrap_or(stripped);
        Some(PathBuf::from(path_str.replace('/', "\\")))
    }
    #[cfg(not(target_os = "windows"))]
    {
        Some(PathBuf::from(stripped))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_info_new() {
        let r = RootInfo::new("file:///home/user/proj");
        assert_eq!(r.uri, "file:///home/user/proj");
        assert!(r.name.is_none());
    }

    #[test]
    fn root_info_with_name() {
        let r = RootInfo::with_name("file:///proj", "my-project");
        assert_eq!(r.name.as_deref(), Some("my-project"));
    }

    #[test]
    fn root_info_serde_roundtrip() {
        let r = RootInfo::with_name("file:///proj", "test");
        let json = serde_json::to_string(&r).unwrap();
        let back: RootInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn root_info_serde_no_name() {
        let r = RootInfo::new("file:///proj");
        let json = serde_json::to_string(&r).unwrap();
        assert!(!json.contains("name"));
        let back: RootInfo = serde_json::from_str(&json).unwrap();
        assert!(back.name.is_none());
    }

    #[test]
    fn registry_add_and_list() {
        let mut reg = RootRegistry::new();
        assert!(reg.add(RootInfo::new("file:///a")));
        assert!(reg.add(RootInfo::new("file:///b")));
        assert_eq!(reg.len(), 2);
        assert!(!reg.is_empty());
    }

    #[test]
    fn registry_add_duplicate_rejected() {
        let mut reg = RootRegistry::new();
        assert!(reg.add(RootInfo::new("file:///a")));
        assert!(!reg.add(RootInfo::new("file:///a")));
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn registry_remove() {
        let mut reg = RootRegistry::new();
        reg.add(RootInfo::new("file:///a"));
        reg.add(RootInfo::new("file:///b"));
        assert!(reg.remove("file:///a"));
        assert_eq!(reg.len(), 1);
        assert!(!reg.remove("file:///nonexistent"));
    }

    #[test]
    fn registry_set_roots() {
        let mut reg = RootRegistry::new();
        reg.add(RootInfo::new("file:///old"));
        reg.set_roots(vec![
            RootInfo::new("file:///new1"),
            RootInfo::new("file:///new2"),
        ]);
        assert_eq!(reg.len(), 2);
        assert_eq!(reg.list()[0].uri, "file:///new1");
    }

    #[test]
    fn registry_version_increments() {
        let mut reg = RootRegistry::new();
        assert_eq!(reg.version(), 0);
        reg.add(RootInfo::new("file:///a"));
        assert_eq!(reg.version(), 1);
        reg.add(RootInfo::new("file:///b"));
        assert_eq!(reg.version(), 2);
        reg.remove("file:///a");
        assert_eq!(reg.version(), 3);
        reg.set_roots(vec![]);
        assert_eq!(reg.version(), 4);
    }

    #[test]
    fn registry_version_no_change_on_noop() {
        let mut reg = RootRegistry::new();
        reg.add(RootInfo::new("file:///a"));
        let v = reg.version();
        reg.add(RootInfo::new("file:///a")); // duplicate, no change
        assert_eq!(reg.version(), v);
        reg.remove("file:///nonexistent"); // not found, no change
        assert_eq!(reg.version(), v);
    }

    #[test]
    fn path_to_file_uri_unix_style() {
        let uri = path_to_file_uri(Path::new("/home/user/project"));
        assert!(uri.starts_with("file://"));
        assert!(uri.contains("/home/user/project"));
    }

    #[test]
    fn file_uri_roundtrip() {
        let original = Path::new("/tmp/test");
        let uri = path_to_file_uri(original);
        let back = file_uri_to_path(&uri);
        assert!(back.is_some());
    }

    #[test]
    fn file_uri_to_path_invalid() {
        assert!(file_uri_to_path("http://example.com").is_none());
        assert!(file_uri_to_path("not-a-uri").is_none());
    }

    #[test]
    fn root_info_from_path() {
        let r = RootInfo::from_path(Path::new("/home/user/my-project"));
        assert!(r.uri.starts_with("file://"));
        assert_eq!(r.name.as_deref(), Some("my-project"));
    }

    #[test]
    fn registry_contains_path() {
        let mut reg = RootRegistry::new();
        reg.add(RootInfo::from_path(Path::new("/home/user/proj")));
        // This test is platform-dependent; on the current platform
        // the roundtrip should work for simple absolute paths
        let root_path = reg.list()[0].to_path();
        if let Some(rp) = root_path {
            assert!(reg.contains_path(&rp.join("src/main.rs")));
        }
    }

    #[test]
    fn registry_empty() {
        let reg = RootRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
        assert_eq!(reg.version(), 0);
    }
}

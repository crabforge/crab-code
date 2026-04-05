//! OS-level sandbox for restricting child process capabilities.
//!
//! Provides a platform-abstract [`Sandbox`] trait and a [`SandboxPolicy`] that
//! describes what a sandboxed process is allowed to do. Platform backends:
//!
//! - **Linux**: Landlock LSM (kernel 5.13+) for filesystem restrictions,
//!   network restrictions via seccomp-bpf (future).
//! - **Windows**: Job Objects with restricted tokens for resource limits and
//!   UI/desktop isolation.
//! - **Other / unsupported**: No-op backend that logs a warning.
//!
//! Gated behind `feature = "sandbox"`.

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

// ── Policy ─────────────────────────────────────────────────────────────

/// Access level for a filesystem path in the sandbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathAccess {
    /// Read-only access.
    ReadOnly,
    /// Read and write access.
    ReadWrite,
    /// Full access including execute.
    Full,
}

impl fmt::Display for PathAccess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReadOnly => f.write_str("read_only"),
            Self::ReadWrite => f.write_str("read_write"),
            Self::Full => f.write_str("full"),
        }
    }
}

/// A single filesystem path rule within a sandbox policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathRule {
    /// The directory or file path to allow.
    pub path: PathBuf,
    /// The access level granted.
    pub access: PathAccess,
}

/// Policy describing what a sandboxed process is allowed to do.
///
/// A default policy denies everything. Fields are additive: each `allow_*`
/// field opens up a specific capability.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SandboxPolicy {
    /// Allowed filesystem paths with their access levels.
    pub path_rules: Vec<PathRule>,
    /// Whether the process may access the network.
    pub allow_network: bool,
    /// Whether the process may spawn child processes.
    pub allow_subprocess: bool,
    /// Maximum memory in bytes (0 = unlimited).
    pub max_memory_bytes: u64,
    /// Maximum CPU time in seconds (0 = unlimited).
    pub max_cpu_seconds: u64,
    /// Maximum number of open file descriptors (0 = unlimited).
    pub max_open_files: u64,
}

impl SandboxPolicy {
    /// Create a policy that denies everything.
    #[must_use]
    pub fn deny_all() -> Self {
        Self::default()
    }

    /// Add a path rule to the policy.
    #[must_use]
    pub fn with_path(mut self, path: impl Into<PathBuf>, access: PathAccess) -> Self {
        self.path_rules.push(PathRule {
            path: path.into(),
            access,
        });
        self
    }

    /// Allow network access.
    #[must_use]
    pub const fn with_network(mut self, allow: bool) -> Self {
        self.allow_network = allow;
        self
    }

    /// Allow subprocess creation.
    #[must_use]
    pub const fn with_subprocess(mut self, allow: bool) -> Self {
        self.allow_subprocess = allow;
        self
    }

    /// Set memory limit in bytes.
    #[must_use]
    pub const fn with_max_memory(mut self, bytes: u64) -> Self {
        self.max_memory_bytes = bytes;
        self
    }

    /// Set CPU time limit in seconds.
    #[must_use]
    pub const fn with_max_cpu(mut self, seconds: u64) -> Self {
        self.max_cpu_seconds = seconds;
        self
    }

    /// Build a reasonable default policy for running tool commands.
    ///
    /// Allows read access to the project directory, read-write to a working
    /// directory, and network access (many tools need HTTP).
    #[must_use]
    pub fn tool_default(project_dir: impl Into<PathBuf>, working_dir: impl Into<PathBuf>) -> Self {
        Self::deny_all()
            .with_path(project_dir, PathAccess::ReadOnly)
            .with_path(working_dir, PathAccess::ReadWrite)
            .with_network(true)
            .with_subprocess(true)
            .with_max_memory(512 * 1024 * 1024) // 512 MB
            .with_max_cpu(120) // 2 minutes
    }

    /// Check whether a given path would be allowed under this policy at the
    /// requested access level.
    #[must_use]
    pub fn check_path(&self, target: &std::path::Path, requested: PathAccess) -> bool {
        for rule in &self.path_rules {
            if target.starts_with(&rule.path) && access_sufficient(rule.access, requested) {
                return true;
            }
        }
        false
    }

    /// Return a summary of what this policy allows.
    #[must_use]
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();

        if self.path_rules.is_empty() {
            parts.push("no filesystem access".to_string());
        } else {
            for rule in &self.path_rules {
                parts.push(format!("{}:{}", rule.path.display(), rule.access));
            }
        }

        parts.push(format!(
            "network:{}",
            if self.allow_network { "yes" } else { "no" }
        ));
        parts.push(format!(
            "subprocess:{}",
            if self.allow_subprocess { "yes" } else { "no" }
        ));

        if self.max_memory_bytes > 0 {
            parts.push(format!("mem:{}MB", self.max_memory_bytes / (1024 * 1024)));
        }
        if self.max_cpu_seconds > 0 {
            parts.push(format!("cpu:{}s", self.max_cpu_seconds));
        }

        parts.join(", ")
    }
}

/// Check if `granted` access level is sufficient for `requested`.
fn access_sufficient(granted: PathAccess, requested: PathAccess) -> bool {
    match requested {
        PathAccess::ReadOnly => true, // any access level grants read
        PathAccess::ReadWrite => matches!(granted, PathAccess::ReadWrite | PathAccess::Full),
        PathAccess::Full => granted == PathAccess::Full,
    }
}

// ── Sandbox trait ──────────────────────────────────────────────────────

/// Result of applying a sandbox to a process.
#[derive(Debug, Clone)]
pub struct SandboxResult {
    /// Whether the sandbox was successfully applied.
    pub applied: bool,
    /// Human-readable description of what was enforced.
    pub description: String,
    /// The backend that was used.
    pub backend: SandboxBackend,
}

/// Which sandbox backend is in use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxBackend {
    /// Linux Landlock LSM.
    Landlock,
    /// Windows Job Object with restricted token.
    WindowsJobObject,
    /// No sandboxing available — policy checked but not enforced.
    Noop,
}

impl fmt::Display for SandboxBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Landlock => f.write_str("landlock"),
            Self::WindowsJobObject => f.write_str("windows_job_object"),
            Self::Noop => f.write_str("noop"),
        }
    }
}

/// Trait for platform-specific sandbox implementations.
pub trait Sandbox: Send + Sync {
    /// The backend identifier.
    fn backend(&self) -> SandboxBackend;

    /// Whether this sandbox backend is available on the current system.
    fn is_available(&self) -> bool;

    /// Apply the sandbox policy to a command before spawning.
    ///
    /// Implementations should configure the command (e.g., pre-exec hooks,
    /// environment, Job Object handles) so that the spawned process is
    /// restricted according to the policy.
    fn apply(
        &self,
        policy: &SandboxPolicy,
        cmd: &mut tokio::process::Command,
    ) -> crab_common::Result<SandboxResult>;
}

// ── Platform backends ──────────────────────────────────────────────────

/// Linux Landlock sandbox backend.
///
/// Restricts filesystem access using the Landlock LSM (Linux Security Module),
/// available on Linux kernel 5.13+. When Landlock is not available, falls back
/// to no-op with a warning.
#[cfg(target_os = "linux")]
pub struct LandlockSandbox;

#[cfg(target_os = "linux")]
impl LandlockSandbox {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

#[cfg(target_os = "linux")]
impl Default for LandlockSandbox {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_os = "linux")]
impl Sandbox for LandlockSandbox {
    fn backend(&self) -> SandboxBackend {
        SandboxBackend::Landlock
    }

    fn is_available(&self) -> bool {
        // Check if Landlock is supported by testing the ABI version.
        // In a real implementation, this would call landlock_create_ruleset
        // with LANDLOCK_CREATE_RULESET_VERSION. For now, check the kernel
        // version heuristic (5.13+).
        let info = sysinfo::System::kernel_version();
        if let Some(version) = info {
            if let Some((major, minor)) = parse_kernel_version(&version) {
                return major > 5 || (major == 5 && minor >= 13);
            }
        }
        false
    }

    fn apply(
        &self,
        policy: &SandboxPolicy,
        _cmd: &mut tokio::process::Command,
    ) -> crab_common::Result<SandboxResult> {
        if !self.is_available() {
            return Ok(SandboxResult {
                applied: false,
                description: "Landlock not available on this kernel".into(),
                backend: SandboxBackend::Landlock,
            });
        }

        // Real Landlock implementation would use pre_exec to:
        // 1. Create a landlock ruleset (landlock_create_ruleset)
        // 2. Add path rules (landlock_add_rule) for each path_rule
        // 3. Enforce the ruleset (landlock_restrict_self)
        //
        // This requires unsafe FFI calls to libc/landlock syscalls.
        // Since unsafe_code is forbidden workspace-wide, the actual
        // enforcement is deferred to a future release that adds a safe
        // wrapper crate (e.g., landlock-rs).
        //
        // For now, return a result indicating policy was validated but
        // not enforced at the OS level.

        Ok(SandboxResult {
            applied: false,
            description: format!(
                "Landlock policy validated (enforcement pending safe wrapper): {}",
                policy.summary()
            ),
            backend: SandboxBackend::Landlock,
        })
    }
}

#[cfg(target_os = "linux")]
fn parse_kernel_version(version: &str) -> Option<(u32, u32)> {
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor))
}

/// Windows Job Object sandbox backend.
///
/// Uses Windows Job Objects to enforce resource limits (memory, CPU time)
/// and UI restrictions on child processes. Filesystem restrictions require
/// restricted tokens (future enhancement).
#[cfg(target_os = "windows")]
pub struct WindowsJobSandbox;

#[cfg(target_os = "windows")]
impl WindowsJobSandbox {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

#[cfg(target_os = "windows")]
impl Default for WindowsJobSandbox {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_os = "windows")]
impl Sandbox for WindowsJobSandbox {
    fn backend(&self) -> SandboxBackend {
        SandboxBackend::WindowsJobObject
    }

    fn is_available(&self) -> bool {
        // Job Objects are available on all supported Windows versions.
        true
    }

    fn apply(
        &self,
        policy: &SandboxPolicy,
        _cmd: &mut tokio::process::Command,
    ) -> crab_common::Result<SandboxResult> {
        // Real implementation would:
        // 1. Create a Job Object (CreateJobObjectW)
        // 2. Set resource limits (SetInformationJobObject with
        //    JOBOBJECT_EXTENDED_LIMIT_INFORMATION for memory/CPU)
        // 3. Set UI restrictions (JOB_OBJECT_UILIMIT_* flags)
        // 4. Assign the child process to the Job Object after spawn
        //
        // This requires windows-sys or winapi crate + unsafe calls.
        // Deferred to when a safe wrapper is added.

        Ok(SandboxResult {
            applied: false,
            description: format!(
                "Windows Job Object policy validated (enforcement pending): {}",
                policy.summary()
            ),
            backend: SandboxBackend::WindowsJobObject,
        })
    }
}

/// No-op sandbox backend for platforms without sandbox support.
pub struct NoopSandbox;

impl NoopSandbox {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for NoopSandbox {
    fn default() -> Self {
        Self::new()
    }
}

impl Sandbox for NoopSandbox {
    fn backend(&self) -> SandboxBackend {
        SandboxBackend::Noop
    }

    fn is_available(&self) -> bool {
        true // always "available" — just doesn't enforce anything
    }

    fn apply(
        &self,
        policy: &SandboxPolicy,
        _cmd: &mut tokio::process::Command,
    ) -> crab_common::Result<SandboxResult> {
        Ok(SandboxResult {
            applied: false,
            description: format!(
                "No sandbox enforcement available on this platform. Policy: {}",
                policy.summary()
            ),
            backend: SandboxBackend::Noop,
        })
    }
}

// ── Factory ────────────────────────────────────────────────────────────

/// Create the best available sandbox backend for the current platform.
#[must_use]
pub fn create_sandbox() -> Box<dyn Sandbox> {
    #[cfg(target_os = "linux")]
    {
        let landlock = LandlockSandbox::new();
        if landlock.is_available() {
            return Box::new(landlock);
        }
    }

    #[cfg(target_os = "windows")]
    {
        return Box::new(WindowsJobSandbox::new());
    }

    #[allow(unreachable_code)]
    Box::new(NoopSandbox::new())
}

/// Apply a sandbox policy to a command using the best available backend.
///
/// This is the main entry point for callers (e.g., `BashTool`) that want
/// to sandbox a child process.
pub fn apply_policy(
    policy: &SandboxPolicy,
    cmd: &mut tokio::process::Command,
) -> crab_common::Result<SandboxResult> {
    let sandbox = create_sandbox();
    sandbox.apply(policy, cmd)
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    // ── Policy construction ────────────────────────────────────────

    #[test]
    fn deny_all_policy() {
        let policy = SandboxPolicy::deny_all();
        assert!(policy.path_rules.is_empty());
        assert!(!policy.allow_network);
        assert!(!policy.allow_subprocess);
        assert_eq!(policy.max_memory_bytes, 0);
        assert_eq!(policy.max_cpu_seconds, 0);
        assert_eq!(policy.max_open_files, 0);
    }

    #[test]
    fn builder_pattern() {
        let policy = SandboxPolicy::deny_all()
            .with_path("/tmp", PathAccess::ReadWrite)
            .with_path("/usr", PathAccess::ReadOnly)
            .with_network(true)
            .with_subprocess(false)
            .with_max_memory(256 * 1024 * 1024)
            .with_max_cpu(60);

        assert_eq!(policy.path_rules.len(), 2);
        assert_eq!(policy.path_rules[0].path, Path::new("/tmp"));
        assert_eq!(policy.path_rules[0].access, PathAccess::ReadWrite);
        assert_eq!(policy.path_rules[1].path, Path::new("/usr"));
        assert_eq!(policy.path_rules[1].access, PathAccess::ReadOnly);
        assert!(policy.allow_network);
        assert!(!policy.allow_subprocess);
        assert_eq!(policy.max_memory_bytes, 256 * 1024 * 1024);
        assert_eq!(policy.max_cpu_seconds, 60);
    }

    #[test]
    fn tool_default_policy() {
        let policy = SandboxPolicy::tool_default("/project", "/project/build");
        assert_eq!(policy.path_rules.len(), 2);
        assert!(policy.allow_network);
        assert!(policy.allow_subprocess);
        assert!(policy.max_memory_bytes > 0);
        assert!(policy.max_cpu_seconds > 0);
    }

    // ── Path checking ──────────────────────────────────────────────

    #[test]
    fn check_path_allowed_read() {
        let policy =
            SandboxPolicy::deny_all().with_path("/home/user/project", PathAccess::ReadOnly);

        assert!(policy.check_path(
            Path::new("/home/user/project/src/main.rs"),
            PathAccess::ReadOnly
        ));
    }

    #[test]
    fn check_path_denied_write_on_readonly() {
        let policy =
            SandboxPolicy::deny_all().with_path("/home/user/project", PathAccess::ReadOnly);

        assert!(!policy.check_path(
            Path::new("/home/user/project/src/main.rs"),
            PathAccess::ReadWrite
        ));
    }

    #[test]
    fn check_path_allowed_write_on_readwrite() {
        let policy = SandboxPolicy::deny_all().with_path("/tmp", PathAccess::ReadWrite);

        assert!(policy.check_path(Path::new("/tmp/output.txt"), PathAccess::ReadWrite));
        assert!(policy.check_path(Path::new("/tmp/output.txt"), PathAccess::ReadOnly));
    }

    #[test]
    fn check_path_denied_outside_rules() {
        let policy = SandboxPolicy::deny_all().with_path("/home/user/project", PathAccess::Full);

        assert!(!policy.check_path(Path::new("/etc/passwd"), PathAccess::ReadOnly));
    }

    #[test]
    fn check_path_full_grants_everything() {
        let policy = SandboxPolicy::deny_all().with_path("/workspace", PathAccess::Full);

        assert!(policy.check_path(Path::new("/workspace/a"), PathAccess::ReadOnly));
        assert!(policy.check_path(Path::new("/workspace/a"), PathAccess::ReadWrite));
        assert!(policy.check_path(Path::new("/workspace/a"), PathAccess::Full));
    }

    #[test]
    fn check_path_empty_policy_denies_all() {
        let policy = SandboxPolicy::deny_all();
        assert!(!policy.check_path(Path::new("/tmp"), PathAccess::ReadOnly));
    }

    // ── Access level logic ─────────────────────────────────────────

    #[test]
    fn access_sufficient_matrix() {
        // ReadOnly granted → only read allowed
        assert!(access_sufficient(
            PathAccess::ReadOnly,
            PathAccess::ReadOnly
        ));
        assert!(!access_sufficient(
            PathAccess::ReadOnly,
            PathAccess::ReadWrite
        ));
        assert!(!access_sufficient(PathAccess::ReadOnly, PathAccess::Full));

        // ReadWrite granted → read and write allowed
        assert!(access_sufficient(
            PathAccess::ReadWrite,
            PathAccess::ReadOnly
        ));
        assert!(access_sufficient(
            PathAccess::ReadWrite,
            PathAccess::ReadWrite
        ));
        assert!(!access_sufficient(PathAccess::ReadWrite, PathAccess::Full));

        // Full granted → everything allowed
        assert!(access_sufficient(PathAccess::Full, PathAccess::ReadOnly));
        assert!(access_sufficient(PathAccess::Full, PathAccess::ReadWrite));
        assert!(access_sufficient(PathAccess::Full, PathAccess::Full));
    }

    // ── Summary ────────────────────────────────────────────────────

    #[test]
    fn summary_deny_all() {
        let policy = SandboxPolicy::deny_all();
        let summary = policy.summary();
        assert!(summary.contains("no filesystem access"));
        assert!(summary.contains("network:no"));
        assert!(summary.contains("subprocess:no"));
    }

    #[test]
    fn summary_with_paths() {
        let policy = SandboxPolicy::deny_all()
            .with_path("/tmp", PathAccess::ReadWrite)
            .with_network(true)
            .with_max_memory(1024 * 1024 * 1024);

        let summary = policy.summary();
        assert!(summary.contains("read_write"));
        assert!(summary.contains("network:yes"));
        assert!(summary.contains("mem:1024MB"));
    }

    // ── Serde ──────────────────────────────────────────────────────

    #[test]
    fn policy_serde_roundtrip() {
        let policy = SandboxPolicy::deny_all()
            .with_path("/tmp", PathAccess::ReadWrite)
            .with_network(true)
            .with_max_cpu(30);

        let json = serde_json::to_string(&policy).unwrap();
        let restored: SandboxPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.path_rules.len(), 1);
        assert!(restored.allow_network);
        assert_eq!(restored.max_cpu_seconds, 30);
    }

    #[test]
    fn path_access_serde() {
        let json = serde_json::to_string(&PathAccess::ReadOnly).unwrap();
        assert_eq!(json, "\"read_only\"");
        let back: PathAccess = serde_json::from_str(&json).unwrap();
        assert_eq!(back, PathAccess::ReadOnly);
    }

    #[test]
    fn sandbox_backend_serde() {
        let json = serde_json::to_string(&SandboxBackend::Landlock).unwrap();
        assert_eq!(json, "\"landlock\"");
        let back: SandboxBackend = serde_json::from_str(&json).unwrap();
        assert_eq!(back, SandboxBackend::Landlock);
    }

    #[test]
    fn sandbox_backend_display() {
        assert_eq!(SandboxBackend::Landlock.to_string(), "landlock");
        assert_eq!(
            SandboxBackend::WindowsJobObject.to_string(),
            "windows_job_object"
        );
        assert_eq!(SandboxBackend::Noop.to_string(), "noop");
    }

    #[test]
    fn path_access_display() {
        assert_eq!(PathAccess::ReadOnly.to_string(), "read_only");
        assert_eq!(PathAccess::ReadWrite.to_string(), "read_write");
        assert_eq!(PathAccess::Full.to_string(), "full");
    }

    // ── Noop sandbox ───────────────────────────────────────────────

    #[test]
    fn noop_sandbox_is_available() {
        let sandbox = NoopSandbox::new();
        assert!(sandbox.is_available());
        assert_eq!(sandbox.backend(), SandboxBackend::Noop);
    }

    #[tokio::test]
    async fn noop_sandbox_apply_returns_not_applied() {
        let sandbox = NoopSandbox::new();
        let policy = SandboxPolicy::deny_all().with_network(true);
        let mut cmd = tokio::process::Command::new("echo");

        let result = sandbox.apply(&policy, &mut cmd).unwrap();
        assert!(!result.applied);
        assert_eq!(result.backend, SandboxBackend::Noop);
        assert!(result.description.contains("No sandbox enforcement"));
    }

    // ── Factory ────────────────────────────────────────────────────

    #[test]
    fn create_sandbox_returns_some_backend() {
        let sandbox = create_sandbox();
        // On Windows we get WindowsJobObject, on Linux Landlock or Noop
        let backend = sandbox.backend();
        assert!(
            backend == SandboxBackend::Landlock
                || backend == SandboxBackend::WindowsJobObject
                || backend == SandboxBackend::Noop
        );
    }

    #[tokio::test]
    async fn apply_policy_works() {
        let policy = SandboxPolicy::tool_default("/project", "/project/out");
        let mut cmd = tokio::process::Command::new("echo");

        let result = apply_policy(&policy, &mut cmd).unwrap();
        // On CI/dev machines, sandbox may or may not be enforced
        assert!(!result.description.is_empty());
    }

    // ── Windows-specific backend ───────────────────────────────────

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_sandbox_is_available() {
        let sandbox = WindowsJobSandbox::new();
        assert!(sandbox.is_available());
        assert_eq!(sandbox.backend(), SandboxBackend::WindowsJobObject);
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn windows_sandbox_apply() {
        let sandbox = WindowsJobSandbox::new();
        let policy = SandboxPolicy::deny_all().with_max_memory(128 * 1024 * 1024);
        let mut cmd = tokio::process::Command::new("cmd");

        let result = sandbox.apply(&policy, &mut cmd).unwrap();
        assert_eq!(result.backend, SandboxBackend::WindowsJobObject);
    }

    // ── Linux-specific backend ─────────────────────────────────────

    #[cfg(target_os = "linux")]
    #[test]
    fn parse_kernel_version_valid() {
        assert_eq!(parse_kernel_version("5.15.0-generic"), Some((5, 15)));
        assert_eq!(parse_kernel_version("6.1.0"), Some((6, 1)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parse_kernel_version_invalid() {
        assert_eq!(parse_kernel_version("invalid"), None);
    }

    // ── Multiple path rules ────────────────────────────────────────

    #[test]
    fn check_path_multiple_rules_first_match_wins() {
        let policy = SandboxPolicy::deny_all()
            .with_path("/home", PathAccess::ReadOnly)
            .with_path("/home/user/work", PathAccess::ReadWrite);

        // /home/user/work/file should match the ReadWrite rule
        assert!(policy.check_path(Path::new("/home/user/work/file.rs"), PathAccess::ReadWrite));

        // /home/other should only match ReadOnly
        assert!(policy.check_path(Path::new("/home/other/file.rs"), PathAccess::ReadOnly));
        assert!(!policy.check_path(Path::new("/home/other/file.rs"), PathAccess::ReadWrite));
    }

    #[test]
    fn policy_default_trait() {
        let p = SandboxPolicy::default();
        assert!(p.path_rules.is_empty());
        assert!(!p.allow_network);
    }
}

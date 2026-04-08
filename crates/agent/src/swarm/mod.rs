//! Swarm/team module for multi-agent orchestration.
//!
//! Provides backend-agnostic sub-agent ("teammate") management. Two backends
//! are included:
//!
//! - [`InProcessBackend`] — tokio tasks with mpsc channels (for testing and
//!   single-process deployments)
//! - [`TmuxBackend`] — spawns each teammate in a tmux pane, communicating
//!   via `tmux send-keys` / `capture-pane`
//!
//! Supporting modules handle tmux pane management ([`pane_manager`]),
//! cross-agent permission synchronization ([`permission_sync`]), and
//! init-script generation ([`init_script`]).

pub mod backend;
pub mod init_script;
pub mod pane_manager;
pub mod permission_sync;
pub mod teammate;

pub use backend::{InProcessBackend, SwarmBackend, TmuxBackend};
pub use init_script::generate_init_script;
pub use pane_manager::{PaneInfo, PaneManager};
pub use permission_sync::{PermissionDecisionEvent, PermissionSyncManager};
pub use teammate::{Teammate, TeammateConfig, TeammateState};

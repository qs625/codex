//! Lightweight rollout path and configuration API shared by rollout consumers.

mod fork_snapshot;
mod truncation;

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

pub use fork_snapshot::ForkSnapshot;
pub use fork_snapshot::InterruptedTurnHistoryMarker;
pub use fork_snapshot::SnapshotTurnState;
pub use fork_snapshot::TurnAborted;
pub use fork_snapshot::append_interrupted_boundary;
pub use fork_snapshot::fork_history_from_snapshot;
pub use fork_snapshot::interrupted_turn_history_marker;
pub use fork_snapshot::snapshot_turn_state;
pub use fork_snapshot::truncate_before_nth_user_message;
pub use truncation::fork_turn_positions_in_rollout;
pub use truncation::initial_history_has_prior_user_turns;
pub use truncation::truncate_rollout_before_nth_user_message_from_start;
pub use truncation::truncate_rollout_to_last_n_fork_turns;
pub use truncation::user_message_positions_in_rollout;

pub const SESSIONS_SUBDIR: &str = "sessions";
pub const ARCHIVED_SESSIONS_SUBDIR: &str = "archived_sessions";

/// Read-only rollout configuration view used by host crates to construct rollout
/// services without depending on the full rollout implementation.
pub trait RolloutConfigView {
    fn codex_home(&self) -> &Path;
    fn sqlite_home(&self) -> &Path;
    fn cwd(&self) -> &Path;
    fn model_provider_id(&self) -> &str;
    fn generate_memories(&self) -> bool;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RolloutConfig {
    pub codex_home: PathBuf,
    pub sqlite_home: PathBuf,
    pub cwd: PathBuf,
    pub model_provider_id: String,
    pub generate_memories: bool,
}

pub type Config = RolloutConfig;

impl RolloutConfig {
    pub fn from_view(view: &impl RolloutConfigView) -> Self {
        Self {
            codex_home: view.codex_home().to_path_buf(),
            sqlite_home: view.sqlite_home().to_path_buf(),
            cwd: view.cwd().to_path_buf(),
            model_provider_id: view.model_provider_id().to_string(),
            generate_memories: view.generate_memories(),
        }
    }
}

impl RolloutConfigView for RolloutConfig {
    fn codex_home(&self) -> &Path {
        self.codex_home.as_path()
    }

    fn sqlite_home(&self) -> &Path {
        self.sqlite_home.as_path()
    }

    fn cwd(&self) -> &Path {
        self.cwd.as_path()
    }

    fn model_provider_id(&self) -> &str {
        self.model_provider_id.as_str()
    }

    fn generate_memories(&self) -> bool {
        self.generate_memories
    }
}

impl<T: RolloutConfigView + ?Sized> RolloutConfigView for &T {
    fn codex_home(&self) -> &Path {
        (*self).codex_home()
    }

    fn sqlite_home(&self) -> &Path {
        (*self).sqlite_home()
    }

    fn cwd(&self) -> &Path {
        (*self).cwd()
    }

    fn model_provider_id(&self) -> &str {
        (*self).model_provider_id()
    }

    fn generate_memories(&self) -> bool {
        (*self).generate_memories()
    }
}

impl<T: RolloutConfigView + ?Sized> RolloutConfigView for Arc<T> {
    fn codex_home(&self) -> &Path {
        self.as_ref().codex_home()
    }

    fn sqlite_home(&self) -> &Path {
        self.as_ref().sqlite_home()
    }

    fn cwd(&self) -> &Path {
        self.as_ref().cwd()
    }

    fn model_provider_id(&self) -> &str {
        self.as_ref().model_provider_id()
    }

    fn generate_memories(&self) -> bool {
        self.as_ref().generate_memories()
    }
}

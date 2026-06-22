use crate::config::Config;

impl codex_rollout_api::RolloutConfigView for Config {
    fn codex_home(&self) -> &std::path::Path {
        self.codex_home.as_path()
    }

    fn sqlite_home(&self) -> &std::path::Path {
        self.sqlite_home.as_path()
    }

    fn cwd(&self) -> &std::path::Path {
        self.cwd.as_path()
    }

    fn model_provider_id(&self) -> &str {
        self.model_provider_id.as_str()
    }

    fn generate_memories(&self) -> bool {
        self.memories.generate_memories
    }
}

pub(crate) use crate::session_rollout_init_error::map_session_init_error;

pub(crate) mod truncation {
    pub(crate) use codex_rollout_api::truncate_rollout_before_nth_user_message_from_start;
    pub(crate) use codex_rollout_api::user_message_positions_in_rollout;
}

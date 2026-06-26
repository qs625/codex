use std::sync::Arc;

use codex_tool_runtime_api::ApplyPatchEnvironment;

pub(crate) struct CoreApplyPatchEnvironment {
    turn_environment: crate::session::turn_context::TurnEnvironment,
}

impl CoreApplyPatchEnvironment {
    pub(crate) fn new(
        turn_environment: crate::session::turn_context::TurnEnvironment,
    ) -> Arc<Self> {
        Arc::new(Self { turn_environment })
    }
}

impl ApplyPatchEnvironment for CoreApplyPatchEnvironment {
    fn environment_id(&self) -> &str {
        &self.turn_environment.environment_id
    }

    fn filesystem(&self) -> Arc<dyn codex_file_system::ExecutorFileSystem> {
        self.turn_environment.environment.get_filesystem()
    }
}

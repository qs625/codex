use std::sync::Arc;

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

impl codex_tool_runtime_api::ApplyPatchEnvironment for CoreApplyPatchEnvironment {
    fn environment_id(&self) -> &str {
        &self.turn_environment.environment_id
    }

    fn filesystem(&self) -> Arc<dyn codex_file_system::ExecutorFileSystem> {
        self.turn_environment.environment.get_filesystem()
    }
}

impl codex_command_service_api::ApplyPatchEnvironment for CoreApplyPatchEnvironment {
    fn environment_id(&self) -> &str {
        &self.turn_environment.environment_id
    }

    fn filesystem(&self) -> Arc<dyn codex_file_system::ExecutorFileSystem> {
        self.turn_environment.environment.get_filesystem()
    }
}

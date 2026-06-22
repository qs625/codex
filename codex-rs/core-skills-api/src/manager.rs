use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use codex_config_types::SkillsConfig;
use codex_file_system::ExecutorFileSystem;
use codex_plugin_types::PluginSkillRoot;
use codex_utils_absolute_path::AbsolutePathBuf;

use crate::SkillConfigLayerStack;
use crate::SkillLoadOutcome;

pub type SkillsRuntimeFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone)]
pub struct SkillsLoadInput {
    pub cwd: AbsolutePathBuf,
    pub effective_skill_roots: Vec<PluginSkillRoot>,
    pub config_layer_stack: SkillConfigLayerStack,
    pub bundled_skills_enabled: bool,
    pub allowlist_patterns: Option<Vec<String>>,
}

impl SkillsLoadInput {
    pub fn new(
        cwd: AbsolutePathBuf,
        effective_skill_roots: Vec<PluginSkillRoot>,
        config_layer_stack: SkillConfigLayerStack,
        bundled_skills_enabled: bool,
    ) -> Self {
        Self {
            cwd,
            effective_skill_roots,
            config_layer_stack,
            bundled_skills_enabled,
            allowlist_patterns: None,
        }
    }

    pub fn with_allowlist_patterns(mut self, allowlist_patterns: Option<Vec<String>>) -> Self {
        self.allowlist_patterns = allowlist_patterns;
        self
    }
}

/// Host-provided runtime for loading and caching skills.
///
/// Implementations own filesystem discovery, bundled skill installation, config-rule evaluation,
/// and cache invalidation. Session/runtime crates should depend on this trait and the lightweight
/// skill context types instead of depending on the concrete loader implementation.
pub trait SkillsRuntime: Send + Sync {
    fn skills_for_config<'a>(
        &'a self,
        input: &'a SkillsLoadInput,
        fs: Option<Arc<dyn ExecutorFileSystem>>,
    ) -> SkillsRuntimeFuture<'a, SkillLoadOutcome>;

    fn skills_for_cwd<'a>(
        &'a self,
        input: &'a SkillsLoadInput,
        force_reload: bool,
        fs: Option<Arc<dyn ExecutorFileSystem>>,
    ) -> SkillsRuntimeFuture<'a, SkillLoadOutcome>;

    fn skill_root_paths_for_config<'a>(
        &'a self,
        input: &'a SkillsLoadInput,
        fs: Option<Arc<dyn ExecutorFileSystem>>,
    ) -> SkillsRuntimeFuture<'a, Vec<AbsolutePathBuf>>;

    fn clear_cache(&self);
}

pub type SharedSkillsRuntime = Arc<dyn SkillsRuntime>;

pub struct DisabledSkillsRuntime;

impl SkillsRuntime for DisabledSkillsRuntime {
    fn skills_for_config<'a>(
        &'a self,
        _input: &'a SkillsLoadInput,
        _fs: Option<Arc<dyn ExecutorFileSystem>>,
    ) -> SkillsRuntimeFuture<'a, SkillLoadOutcome> {
        Box::pin(async { SkillLoadOutcome::default() })
    }

    fn skills_for_cwd<'a>(
        &'a self,
        _input: &'a SkillsLoadInput,
        _force_reload: bool,
        _fs: Option<Arc<dyn ExecutorFileSystem>>,
    ) -> SkillsRuntimeFuture<'a, SkillLoadOutcome> {
        Box::pin(async { SkillLoadOutcome::default() })
    }

    fn skill_root_paths_for_config<'a>(
        &'a self,
        _input: &'a SkillsLoadInput,
        _fs: Option<Arc<dyn ExecutorFileSystem>>,
    ) -> SkillsRuntimeFuture<'a, Vec<AbsolutePathBuf>> {
        Box::pin(async { Vec::new() })
    }

    fn clear_cache(&self) {}
}

pub fn bundled_skills_enabled_from_stack(config_layer_stack: &SkillConfigLayerStack) -> bool {
    let effective_config = config_layer_stack.effective_config();
    let Some(skills_value) = effective_config
        .as_table()
        .and_then(|table| table.get("skills"))
    else {
        return true;
    };

    let skills: SkillsConfig = match skills_value.clone().try_into() {
        Ok(skills) => skills,
        Err(err) => {
            tracing::warn!("invalid skills config: {err}");
            return true;
        }
    };

    skills.bundled.unwrap_or_default().enabled
}

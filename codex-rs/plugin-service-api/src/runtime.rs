use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use codex_config_types::McpServerConfig;

use crate::AppConnectorId;
use crate::PluginAgentDir;
use crate::PluginCapabilitySummary;
use crate::PluginHookSource;
use crate::PluginSkillRoot;
use crate::PluginsConfigInput;

pub type PluginLoadOutcome = crate::load_outcome::PluginLoadOutcome<McpServerConfig>;

pub type PluginRuntimeFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSuggestDiscoverablePlugin {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub has_skills: bool,
    pub mcp_server_names: Vec<String>,
    pub app_connector_ids: Vec<String>,
}

/// Host-provided plugin runtime used by core/session code that needs loaded plugin capabilities.
///
/// Implementations own the concrete plugin cache, filesystem access, and loading mechanics. Callers
/// should depend on this trait when they only need enabled plugin capabilities or cache invalidation,
/// and leave install/list/read marketplace management to the full plugin manager boundary.
pub trait PluginRuntime: Send + Sync {
    fn plugins_for_config<'a>(
        &'a self,
        config: &'a PluginsConfigInput,
    ) -> PluginRuntimeFuture<'a, PluginLoadOutcome>;

    fn is_configured_plugin_installed(&self, config: &PluginsConfigInput, plugin_id: &str) -> bool;

    fn list_tool_suggest_discoverable_plugins<'a>(
        &'a self,
        config: &'a PluginsConfigInput,
        configured_plugin_ids: &'a std::collections::HashSet<String>,
        disabled_plugin_ids: &'a std::collections::HashSet<String>,
    ) -> PluginRuntimeFuture<'a, Result<Vec<ToolSuggestDiscoverablePlugin>, String>>;

    fn clear_cache(&self);

    fn capability_summaries_for_config<'a>(
        &'a self,
        config: &'a PluginsConfigInput,
    ) -> PluginRuntimeFuture<'a, Vec<PluginCapabilitySummary>> {
        Box::pin(async move {
            self.plugins_for_config(config)
                .await
                .capability_summaries()
                .to_vec()
        })
    }

    fn effective_apps_for_config<'a>(
        &'a self,
        config: &'a PluginsConfigInput,
    ) -> PluginRuntimeFuture<'a, Vec<AppConnectorId>> {
        Box::pin(async move { self.plugins_for_config(config).await.effective_apps() })
    }

    fn effective_skill_roots_for_config<'a>(
        &'a self,
        config: &'a PluginsConfigInput,
    ) -> PluginRuntimeFuture<'a, Vec<PluginSkillRoot>> {
        Box::pin(async move {
            self.plugins_for_config(config)
                .await
                .effective_plugin_skill_roots()
        })
    }

    fn plugin_hook_sources_for_config<'a>(
        &'a self,
        config: &'a PluginsConfigInput,
        plugin_hooks_enabled: bool,
    ) -> PluginRuntimeFuture<'a, (Vec<PluginHookSource>, Vec<String>)> {
        Box::pin(async move {
            if !plugin_hooks_enabled {
                return (Vec::new(), Vec::new());
            }
            let outcome = self.plugins_for_config(config).await;
            (
                outcome.effective_plugin_hook_sources(),
                outcome.effective_plugin_hook_warnings(),
            )
        })
    }

    fn plugin_agent_dirs_for_config<'a>(
        &'a self,
        config: &'a PluginsConfigInput,
    ) -> PluginRuntimeFuture<'a, Vec<PluginAgentDir>> {
        Box::pin(async move {
            self.plugins_for_config(config)
                .await
                .effective_plugin_agent_dirs()
        })
    }

    fn connector_ids_for_config<'a>(
        &'a self,
        config: &'a PluginsConfigInput,
    ) -> PluginRuntimeFuture<'a, HashSet<String>> {
        Box::pin(async move {
            self.capability_summaries_for_config(config)
                .await
                .into_iter()
                .flat_map(|plugin| plugin.app_connector_ids.into_iter())
                .map(|connector_id| connector_id.0)
                .collect()
        })
    }
}

pub type SharedPluginRuntime = Arc<dyn PluginRuntime>;

#[derive(Debug, Default)]
pub struct DisabledPluginRuntime;

impl PluginRuntime for DisabledPluginRuntime {
    fn plugins_for_config<'a>(
        &'a self,
        _config: &'a PluginsConfigInput,
    ) -> PluginRuntimeFuture<'a, PluginLoadOutcome> {
        Box::pin(async { PluginLoadOutcome::default() })
    }

    fn is_configured_plugin_installed(
        &self,
        _config: &PluginsConfigInput,
        _plugin_id: &str,
    ) -> bool {
        false
    }

    fn list_tool_suggest_discoverable_plugins<'a>(
        &'a self,
        _config: &'a PluginsConfigInput,
        _configured_plugin_ids: &'a std::collections::HashSet<String>,
        _disabled_plugin_ids: &'a std::collections::HashSet<String>,
    ) -> PluginRuntimeFuture<'a, Result<Vec<ToolSuggestDiscoverablePlugin>, String>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn clear_cache(&self) {}
}

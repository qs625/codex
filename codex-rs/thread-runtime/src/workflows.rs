use crate::config::Config;
use codex_config_state::ConfigLayerEntry;
use codex_config_state::ConfigLayerStackOrdering;
use codex_config_types::ConfigLayerSource;
pub use codex_workflow_api::WorkflowDetails;
pub use codex_workflow_api::WorkflowDiagnostic;
pub use codex_workflow_api::WorkflowInputSpec;
pub use codex_workflow_api::WorkflowManifest;
pub use codex_workflow_api::WorkflowRegistry;
pub use codex_workflow_api::WorkflowSource;
pub use codex_workflow_api::WorkflowSummary;
pub use codex_workflow_api::render_available_workflows_body;
use std::path::PathBuf;

pub fn load_workflow_registry(config: &Config) -> WorkflowRegistry {
    codex_workflow_api::load_workflow_registry_from_roots(
        config.codex_home.join("workflows").to_path_buf(),
        project_workflow_roots(config),
    )
}

fn project_workflow_roots(config: &Config) -> Vec<PathBuf> {
    let layers = config.config_layer_stack.get_layers(
        ConfigLayerStackOrdering::LowestPrecedenceFirst,
        /*include_disabled*/ false,
    );
    if layers.is_empty() {
        return vec![config.cwd.join(".codex").join("workflows").to_path_buf()];
    }

    let roots = layers
        .into_iter()
        .filter(|layer| matches!(&layer.name, ConfigLayerSource::Project { .. }))
        .filter_map(ConfigLayerEntry::config_folder)
        .map(|folder| folder.join("workflows").to_path_buf())
        .collect::<Vec<_>>();
    if roots.is_empty() {
        vec![config.cwd.join(".codex").join("workflows").to_path_buf()]
    } else {
        roots
    }
}

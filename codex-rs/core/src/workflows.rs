use crate::config::Config;
pub use codex_workflow::WorkflowDetails;
pub use codex_workflow::WorkflowDiagnostic;
pub use codex_workflow::WorkflowInputSpec;
pub use codex_workflow::WorkflowManifest;
pub use codex_workflow::WorkflowRegistry;
pub use codex_workflow::WorkflowSource;
pub use codex_workflow::WorkflowSummary;
pub use codex_workflow::render_available_workflows_body;
use codex_config::ConfigLayerSource;
use codex_config::ConfigLayerStackOrdering;
use std::path::PathBuf;

pub fn load_workflow_registry(config: &Config) -> WorkflowRegistry {
    codex_workflow::load_workflow_registry_from_roots(
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
        .filter_map(codex_config::ConfigLayerEntry::config_folder)
        .map(|folder| folder.join("workflows").to_path_buf())
        .collect::<Vec<_>>();
    if roots.is_empty() {
        vec![config.cwd.join(".codex").join("workflows").to_path_buf()]
    } else {
        roots
    }
}

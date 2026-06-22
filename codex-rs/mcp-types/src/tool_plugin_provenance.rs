use std::collections::HashMap;

/// Plugin display-name provenance for connector and MCP server tools.
///
/// The MCP runtime uses this to annotate discovered tools, while core and app
/// surfaces can read it without depending on the full MCP runtime crate.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolPluginProvenance {
    plugin_display_names_by_connector_id: HashMap<String, Vec<String>>,
    plugin_display_names_by_mcp_server_name: HashMap<String, Vec<String>>,
}

impl ToolPluginProvenance {
    pub fn from_plugin_sources(
        connector_sources: impl IntoIterator<Item = (String, String)>,
        mcp_server_sources: impl IntoIterator<Item = (String, String)>,
    ) -> Self {
        let mut provenance = Self::default();
        for (connector_id, plugin_display_name) in connector_sources {
            provenance
                .plugin_display_names_by_connector_id
                .entry(connector_id)
                .or_default()
                .push(plugin_display_name);
        }
        for (server_name, plugin_display_name) in mcp_server_sources {
            provenance
                .plugin_display_names_by_mcp_server_name
                .entry(server_name)
                .or_default()
                .push(plugin_display_name);
        }
        provenance.normalize();
        provenance
    }

    pub fn plugin_display_names_for_connector_id(&self, connector_id: &str) -> &[String] {
        self.plugin_display_names_by_connector_id
            .get(connector_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn plugin_display_names_for_mcp_server_name(&self, server_name: &str) -> &[String] {
        self.plugin_display_names_by_mcp_server_name
            .get(server_name)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    fn normalize(&mut self) {
        for plugin_names in self
            .plugin_display_names_by_connector_id
            .values_mut()
            .chain(self.plugin_display_names_by_mcp_server_name.values_mut())
        {
            plugin_names.sort_unstable();
            plugin_names.dedup();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ToolPluginProvenance;

    #[test]
    fn collects_and_deduplicates_plugin_sources() {
        let provenance = ToolPluginProvenance::from_plugin_sources(
            [
                ("connector_example".to_string(), "beta-plugin".to_string()),
                ("connector_example".to_string(), "alpha-plugin".to_string()),
                ("connector_example".to_string(), "alpha-plugin".to_string()),
                ("connector_gmail".to_string(), "beta-plugin".to_string()),
            ],
            [
                ("beta".to_string(), "beta-plugin".to_string()),
                ("alpha".to_string(), "alpha-plugin".to_string()),
            ],
        );

        assert_eq!(
            provenance.plugin_display_names_for_connector_id("connector_example"),
            &["alpha-plugin".to_string(), "beta-plugin".to_string()]
        );
        assert_eq!(
            provenance.plugin_display_names_for_connector_id("connector_gmail"),
            &["beta-plugin".to_string()]
        );
        assert_eq!(
            provenance.plugin_display_names_for_mcp_server_name("alpha"),
            &["alpha-plugin".to_string()]
        );
        assert_eq!(
            provenance.plugin_display_names_for_mcp_server_name("beta"),
            &["beta-plugin".to_string()]
        );
        assert!(
            provenance
                .plugin_display_names_for_connector_id("missing")
                .is_empty()
        );
    }
}

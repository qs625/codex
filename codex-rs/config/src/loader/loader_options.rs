use std::path::Path;
use std::path::PathBuf;

use codex_utils_absolute_path::AbsolutePathBuf;
use protocol::config_types::ProfileV2Name;

const CONFIG_TOML_FILE: &str = "config.toml";

/// User-facing config loading behavior that is not part of the config document.
#[derive(Debug, Default, Clone)]
pub struct ConfigLoadOptions {
    pub loader_overrides: LoaderOverrides,
    pub strict_config: bool,
}

impl From<LoaderOverrides> for ConfigLoadOptions {
    fn from(loader_overrides: LoaderOverrides) -> Self {
        Self {
            loader_overrides,
            strict_config: false,
        }
    }
}

/// LoaderOverrides overrides managed configuration inputs, primarily for tests.
#[derive(Debug, Default, Clone)]
pub struct LoaderOverrides {
    pub user_config_path: Option<AbsolutePathBuf>,
    pub user_config_profile: Option<ProfileV2Name>,
    pub managed_config_path: Option<PathBuf>,
    pub system_config_path: Option<PathBuf>,
    pub system_requirements_path: Option<PathBuf>,
    pub ignore_managed_requirements: bool,
    pub ignore_user_config: bool,
    pub ignore_user_and_project_exec_policy_rules: bool,
    //TODO(gt): Add a macos_ prefix to this field and remove the target_os check.
    #[cfg(target_os = "macos")]
    pub managed_preferences_base64: Option<String>,
    pub macos_managed_config_requirements_base64: Option<String>,
}

impl LoaderOverrides {
    /// Returns overrides that ignore host-managed configuration.
    ///
    /// This is intended for tests that should load only repo-controlled config fixtures.
    pub fn without_managed_config_for_tests() -> Self {
        let base = std::env::temp_dir().join("codex-config-tests");
        Self {
            user_config_path: None,
            user_config_profile: None,
            managed_config_path: Some(base.join("managed_config.toml")),
            system_config_path: Some(base.join(CONFIG_TOML_FILE)),
            system_requirements_path: Some(base.join("requirements.toml")),
            ignore_managed_requirements: false,
            ignore_user_config: false,
            ignore_user_and_project_exec_policy_rules: false,
            #[cfg(target_os = "macos")]
            managed_preferences_base64: Some(String::new()),
            macos_managed_config_requirements_base64: Some(String::new()),
        }
    }

    /// Returns overrides with host MDM disabled and managed config loaded from `managed_config_path`.
    ///
    /// This is intended for tests that supply an explicit managed config fixture.
    pub fn with_managed_config_path_for_tests(managed_config_path: PathBuf) -> Self {
        Self {
            user_config_path: None,
            user_config_profile: None,
            managed_config_path: Some(managed_config_path),
            ..Self::without_managed_config_for_tests()
        }
    }

    pub fn user_config_path(&self, codex_home: &Path) -> std::io::Result<AbsolutePathBuf> {
        match self.user_config_path.as_ref() {
            Some(path) => Ok(path.clone()),
            None => Ok(AbsolutePathBuf::resolve_path_against_base(
                CONFIG_TOML_FILE,
                codex_home,
            )),
        }
    }
}

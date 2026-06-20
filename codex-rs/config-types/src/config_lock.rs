use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ConfigLockfileToml<TConfig> {
    pub version: u32,
    pub codex_version: String,

    /// Replayable effective config captured in the lockfile.
    pub config: TConfig,
}

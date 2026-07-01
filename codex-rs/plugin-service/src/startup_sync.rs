use std::path::Path;
use std::path::PathBuf;

const CURATED_PLUGINS_RELATIVE_DIR: &str = ".tmp/plugins";
const CURATED_PLUGINS_SHA_FILE: &str = ".tmp/plugins.sha";

pub fn curated_plugins_repo_path(codex_home: &Path) -> PathBuf {
    codex_home.join(CURATED_PLUGINS_RELATIVE_DIR)
}

pub fn read_curated_plugins_sha(codex_home: &Path) -> Option<String> {
    read_sha_file(codex_home.join(CURATED_PLUGINS_SHA_FILE).as_path())
}

pub fn has_local_curated_plugins_snapshot(codex_home: &Path) -> bool {
    curated_plugins_repo_path(codex_home)
        .join(".agents/plugins/marketplace.json")
        .is_file()
        && codex_home.join(CURATED_PLUGINS_SHA_FILE).is_file()
}

fn read_sha_file(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|contents| contents.trim().to_string())
        .filter(|contents| !contents.is_empty())
}

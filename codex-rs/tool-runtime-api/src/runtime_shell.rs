use codex_tool_config::ToolUserShellType;
use codex_utils_absolute_path::AbsolutePathBuf;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct RuntimeShellSnapshot {
    pub path: AbsolutePathBuf,
    pub cwd: AbsolutePathBuf,
}

#[derive(Clone, Debug)]
pub struct RuntimeShell {
    pub shell_type: ToolUserShellType,
    pub shell_path: PathBuf,
    pub shell_snapshot: Option<RuntimeShellSnapshot>,
}

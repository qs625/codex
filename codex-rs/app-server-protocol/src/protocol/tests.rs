use super::*;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_absolute_path::test_support::PathBufExt;
use codex_utils_absolute_path::test_support::test_path_buf;
use protocol::approvals::ElicitationRequest as CoreElicitationRequest;
use protocol::models::AdditionalPermissionProfile as CoreAdditionalPermissionProfile;
use protocol::models::ContentItem;
use protocol::models::FileSystemPermissions as CoreFileSystemPermissions;
use protocol::models::ManagedFileSystemPermissions as CoreManagedFileSystemPermissions;
use protocol::models::NetworkPermissions as CoreNetworkPermissions;
use protocol::models::ResponseItem;
use protocol::permissions::FileSystemAccessMode as CoreFileSystemAccessMode;
use protocol::permissions::FileSystemPath as CoreFileSystemPath;
use protocol::permissions::FileSystemSandboxEntry as CoreFileSystemSandboxEntry;
use protocol::permissions::FileSystemSpecialPath as CoreFileSystemSpecialPath;
use protocol::protocol::AgentStatus as CoreAgentStatus;
use protocol::protocol::AskForApproval as CoreAskForApproval;
use protocol::protocol::GranularApprovalConfig as CoreGranularApprovalConfig;
use protocol::protocol::NetworkAccess as CoreNetworkAccess;
use protocol::request_permissions::RequestPermissionProfile as CoreRequestPermissionProfile;
use serde_json::Value as JsonValue;
use serde_json::json;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::path::PathBuf;

mod mcp_and_guardian;
mod permissions_and_sandbox;
mod plugin_and_dynamic_tools;
mod process_and_command;
mod thread_and_turn;

fn absolute_path_string(path: &str) -> String {
    let path = format!("/{}", path.trim_start_matches('/'));
    test_path_buf(&path).display().to_string()
}

fn absolute_path(path: &str) -> AbsolutePathBuf {
    let path = format!("/{}", path.trim_start_matches('/'));
    test_path_buf(&path).abs()
}

fn test_absolute_path() -> AbsolutePathBuf {
    absolute_path("readable")
}

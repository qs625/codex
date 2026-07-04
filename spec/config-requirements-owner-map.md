# config-requirements 归属梳理

## 当前结论

`codex-config-requirements` 当前混合了三类内容：

1. requirements 归一化/合并模型
2. 具体 domain 的业务 DTO
3. 少量已经可以回收到 `config-service` 的加载辅助

其中第 3 类里的 `CloudRequirementsLoader` 已经迁回 `config-service`。

## 建议归属

### 保留在 requirements model crate 的内容

这些类型仍然是 requirements 归一化逻辑自身的核心：

- `ConfigRequirements`
- `ConfigRequirementsToml`
- `ConfigRequirementsWithSources`
- `ConstrainedWithSource<T>`
- `Sourced<T>`
- `RequirementSource`

### 应迁回各 domain service-api 的 DTO

#### 权限 / sandbox / network

更适合 `permissions-service-api` 或对应权限 domain API：

- `NetworkDomainPermissionsToml`
- `NetworkDomainPermissionToml`
- `NetworkUnixSocketPermissionsToml`
- `NetworkUnixSocketPermissionToml`
- `NetworkRequirementsToml`
- `NetworkConstraints`
- `FilesystemRequirementsToml`
- `PermissionsRequirementsToml`
- `FilesystemConstraints`
- `FilesystemDenyReadPattern`
- `SandboxModeRequirement`
- `RemoteSandboxConfigToml`
- `RequirementsExecPolicy*`

#### 插件 / MCP / Apps

更适合 `plugin-service-api` / `mcp-service-api` / app 相关 API：

- `McpServerIdentity`
- `McpServerRequirement`
- `PluginRequirementsToml`
- `AppToolRequirementToml`
- `AppToolsRequirementsToml`
- `AppRequirementToml`
- `AppsRequirementsToml`

#### feature / web search

需要再确认 owner，但不应继续长期留在 config crate：

- `FeatureRequirementsToml`
- `WebSearchModeRequirement`

## 下一步建议

优先顺序：

1. 先迁 `permissions` 这一组，因为已经存在部分重复定义（如 `config-permissions`）
2. 再迁 `plugin / mcp / apps` 这一组
3. 最后再收 `ConfigRequirements*` 主体，让它只依赖 owner API 定义的 DTO

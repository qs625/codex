# codex-tools

`codex-tools` 是 `codex-core` 之外的共享工具基础 crate，负责承载可被多个
crate 复用的 model-visible tool 定义、适配、规划和基础执行契约。

当前这个 crate 拥有原本不需要继续留在 `core/src/tools/spec.rs` 或
`core/src/client_common.rs` 的 host-facing tool 模型和辅助逻辑：

- 聚合模型，例如 `ToolSpec`、`LoadableToolSpec`、`ResponsesApiNamespace` 和
  `ResponsesApiNamespaceTool`
- 组装工具集时使用的 host config 和 discovery 模型，包括 `ToolsConfig`、
  discoverable-tool 模型，以及 request-plugin-install helpers
- host adapters，例如 schema sanitization、MCP/dynamic conversion、
  code-mode augmentation 和 image-detail normalization
- 纯 tool planning，例如 agent tool pattern 过滤、hosted model tool specs、
  namespace 合并，以及 code-mode exec prompt plan
- 共享的 executable-tool 契约，例如 `ToolExecutor`、`ToolCall` 和 `ToolOutput`

这次提取是长期迁移的第一步。目标不是一次性把整个 `core/src/tools` 搬进这个
crate，而是按可 review、可回滚的增量迁出可复用部分；兼容性敏感的编排逻辑在
周边边界稳定前继续留在 `codex-core`。

## 目标边界

后续这个 crate 应继续承载多个消费者共享的 host-side tool 基础设施，例如：

- host-visible aggregate tool models
- tool-set planning 和 discovery helpers
- MCP 与 dynamic-tool 到 Responses API 形状的适配
- 不依赖 `codex-core` 的 code-mode 兼容 shim
- 不依赖 `Session` / `TurnContext` / approval runtime 的内建工具 `ToolSpec`
  构造器，包括 shell-like、workflow、multi-agent、goal、MCP resource、plan、
  request-user-input、plugin-install、test-sync、view-image 和 code-mode spec
- 其他有明确复用方、范围窄的 host utilities

对应的非目标同样重要：

- 不要过早移动 `codex-core` 的 Session/turn orchestration
- 不要把 `Session` / `TurnContext` / approval flow / runtime execution 逻辑拉进
  这个 crate，除非这些依赖已经先拆成稳定共享接口
- 不要把这个 crate 变成无关 helper code 的集合

## 迁移方式

预期迁移形态：

1. extension-owned executable-tool authoring 继续留在 `codex-extension-api`。
2. host-side planning/adaptation helpers 在不再需要耦合 `codex-core` 时迁入这里。
3. compatibility-sensitive adapters 在下游调用方更新前继续留在 `codex-core`。
4. 更高层的 host infrastructure 只有在边界清楚且可独立测试后再提取。

## Crate 约定

这个 crate 应保持比 `core/src/tools` 更严格的结构，避免继续膨胀：

- `src/lib.rs` 保持只做 exports。
- 业务逻辑放在具名模块文件中，例如 `foo.rs`。
- `foo.rs` 的单元测试放在相邻的 `foo_tests.rs`。
- 实现文件使用下面方式挂载测试：

```rust
#[cfg(test)]
#[path = "foo_tests.rs"]
mod tests;
```

如果这个 crate 开始积累需要 `codex-core` runtime state 的代码，应先重新审视拆分边界，
再决定是否继续添加。

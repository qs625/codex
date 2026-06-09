# 移除 wait_agent tool

## 任务 brief

- 用户：使用 multi-agent 协作能力的 Codex agent。
- 问题：`wait_agent` 曾作为等待子 agent 的工具暴露给模型，容易把正常协作流程变成显式等待或轮询。当前期望是子 agent 完成、阻塞或发送消息时自动通知父 agent。
- 成功标准：可用 tool 列表中不再包含 `wait_agent`；提示词和 agent 工作流不再指导调用它；移除不再可达的 handler、schema 和测试；不破坏 child completion 自动通知链路。
- 非目标：不删除旧会话历史中 `wait_agent` tool call 的展示兼容；不修改 `InterAgentCommunication::ChildCompletion`、mailbox 入站、app-server status update 或前端 child completion 展示。

## 技术设计

`wait_agent` 的可见入口集中在 `codex-rs/core/src/tools/spec_plan.rs` 的 collab tool 注册。删除 v1 和 v2 注册后，模型不会再从正常 tool surface 中看到或调用该工具。对应 handler 模块、tool schema helper、timeout 参数注入和 handler/schema 测试随之删除，避免保留不可达配置和编译引用。

child completion 自动通知由 session/mailbox/protocol 链路负责，不依赖 `wait_agent` handler：

- `InterAgentCommunication::ChildCompletion`
- `Session::maybe_notify_parent_of_final_status`
- `Session::forward_child_completion_to_parent`
- mailbox 入站和 live item 分发
- app-server `CollabAgentStatusUpdate` 映射

本改动不触碰这些路径。

## 兼容性

旧配置中的 multi-agent wait timeout 字段暂不从配置结构中移除，避免扩大到配置 schema 和迁移。它们不再进入 `ToolsConfig` 运行时 tool 构建。root-worker prototype 对历史 `wait_agent` tool call 的摘要展示暂保留，用于旧会话回放兼容，不代表该 tool 仍可调用。

## 风险

- 如果未来还有外部客户端依赖 `wait_agent` 调用，会收到未知工具或无法调用；这是本次移除的预期兼容影响。
- `wait_agent` 被移除后，父 agent 必须依赖自动通知和 mailbox 更新继续工作；因此回归验证需要覆盖 tool 列表以及 child completion/mailbox 相关测试。

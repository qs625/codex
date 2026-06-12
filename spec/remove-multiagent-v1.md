# 移除 MultiAgent V1 与 legacy completion watcher

## 任务 brief

MultiAgent 运行时不再通过配置在 V1/V2 之间选择。成功标准：

- collab 工具面只暴露 V2 语义：`spawn_agent`、`followup_task`、`close_agent`、`list_agents`。
- legacy V1 工具 `send_input` / `resume_agent` 以及 `spawn_agent.fork_context` 不再通过工具注册路径暴露。
- child completion 只通过 V2 typed 路径发送，并携带 `AgentStatus`。
- `AgentControl` 不再启动 legacy completion watcher，也不再通过 raw `inject_user_message_without_turn` 注入完成通知。
- 旧 `features.multi_agent_v2.enabled` 配置不再决定是否回落到 V1；保留 `features.multi_agent_v2` 子配置仅作为 V2 参数来源。
- 旧 `agents.max_threads` 作为兼容 alias 继续可用，映射为 V2 session 并发上限中的子 agent 数。

非目标：

- 不重做 root-worker 展示。
- 不删除通用 feature/config 基础设施。
- 不改变 V2 completion gate 的 active/subtree/mailbox/pending direct child 语义。

## 技术设计

工具注册层在 `collab_tools` 打开时始终注册 V2 handler。`ToolsConfig::multi_agent_v2` 只表达当前 collab 工具面是否使用 V2，不再直接反映 `Feature::MultiAgentV2` 的开关状态。

child spawn/resume 后不再为非 V2 child 启动 watcher。direct child completion pending 计数不再检查 feature gate，只根据 child 是否为 management agent 判断，因为所有 thread-spawn child 都应由 V2 final-status 路径负责完成通知。

`Session::maybe_notify_parent_of_final_status_for_source` 不再用 `Feature::MultiAgentV2` 作为早退条件。只要当前 session 是 thread-spawn child、有 agent path、状态 final、且 active gate 允许，就通过 `forward_child_completion_to_parent` 发送带 status 的 typed `InterAgentCommunication::ChildCompletion`。

V2 usage hint 仍沿用 `features.multi_agent_v2` 下的文本配置，但是否启用 hint 不再影响 completion 或工具面是否回落 V1。

`agents.max_threads = N` 继续被配置层接受，并规范化为 `features.multi_agent_v2.max_concurrent_threads_per_session = N + 1`。运行时仍由 `reserve_spawn_slot` 基于 `agent_max_threads = N` 返回明确的 `AgentLimitReached` 限制错误；config lock 只写出 canonical V2 fanout 配置，不再持久化 legacy alias。

`agents.max_depth` 不再通过修改 feature set 隐藏 collab tools。工具面是否暴露只取决于 `Collab`/tool policy；当当前调用会超过深度上限时，V2 `spawn_agent` handler 在调用时返回模型可读错误。

## 风险

仍保留 `features.multi_agent_v2` 配置表用于等待时间、并发数和提示文本参数，因此配置 schema 中不会完全移除该表。剩余风险是旧配置显式关闭 `multi_agent_v2` 的用户仍会看到 V2 工具面；这是本任务要求的行为收敛。旧 `agents.max_threads` 与 V2 fanout 同时配置时，compat alias 以旧字段为准并写出 canonical V2 配置。

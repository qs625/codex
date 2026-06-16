# 恢复 MultiAgent V2 wait_agent tool

## 任务 brief

- 用户：使用 multi-agent 协作能力的 Codex agent。
- 问题：移除 `wait_agent` 后，父 agent 在需要显式等待 subagent 下一条相关 typed IAC、child completion 或 final status 时，只能依赖模型自行 sleep/轮询，容易造成高频空转或错过已经进入 parent pending input 的消息。
- 成功标准：MultiAgent V2 tool surface 重新暴露 `wait_agent`；调用开始先检查 parent pending input/mailbox 中已有匹配消息并立即返回；后续使用 runtime notify/backoff 等待 status 或 mailbox 事件；不 drain/消费 pending 输入；live/history/display 继续走 typed `CollabWaitingBegin/End` 与 `ResponseItem -> ThreadItem` 投影。
- 非目标：不恢复 V1 `send_input`/`resume_agent`；不恢复 legacy completion watcher 或 raw child completion fallback；不从 `<subagent_notification>`、assistant text、legacy JSON envelope 或 raw marker 反解展示；不实现 Dynamic Workflow 后续 runner/persistence/app-server v2 控制面。

## 技术设计

`wait_agent` 的可见入口集中在 `codex-rs/core/src/tools/spec_plan.rs` 的 collab tool 注册。本次只在 MultiAgent V2 工具集中恢复：

- tool spec helper：`create_wait_agent_tool_v2`
- handler：`multi_agents_v2::wait_agent`
- 注册：与 `spawn_agent`、`followup_task`、`close_agent`、`list_agents` 同属 `collab_tools`

等待语义：

- 目标解析复用 V2 `resolve_agent_target`，支持 agent id、canonical task path 和当前 V2 已支持的相对 task path。
- 调用开始先检查目标 agent status；若已 final，立即返回。
- 再检查 parent active turn pending input 与 mailbox buffered input 中的 typed `PendingInputItem::InterAgentCommunication` / `ResponseItem::InterAgentCommunication`，匹配 `author` 或 `sender_thread_id` 指向目标 agent 的消息；若已有匹配消息，立即返回，不 drain。
- 未命中时订阅目标 status watch 与 parent mailbox sequence watch，按 snapshot + notify 模式只等待当前 runtime window 内的后续事件。
- 当前 window 超时后立即返回 `Timeout`，并推进该 sender/receiver target 的 backoff window；下一次同 target `wait_agent` 调用使用推进后的窗口。
- 收到 pending mailbox、mailbox/status event、final status 或 child completion 等相关事件后重置 backoff/window 并返回摘要；如果 mailbox event 不包含匹配消息，本次调用继续等待 current window 的剩余时间，不 reset 也不推进 backoff。
- `features.multi_agent_v2.default_wait_timeout_ms` 作为 initial window，默认 60 秒；`features.multi_agent_v2.max_wait_timeout_ms` 作为 hard cap，默认 30 分钟。工具不暴露 poll interval。
- `CollabWaitingBegin/End.timeout_ms` 面向客户端 wait lifecycle 展示，只表达本次实际等待窗口（current backoff window），不能填入 hard cap；hard cap 仅保留在 `wait_agent` 工具结果的 `hard_cap_timeout_ms` 中，避免 UI 把总上限误显示为本次等待 timeout。工具结果同时返回 `initial_timeout_ms` 和 `current_timeout_ms`，便于模型和客户端区分初始窗口与本次窗口。

## 兼容性

旧配置中的 multi-agent wait timeout 字段继续保留并重新生效，不新增配置项。root-worker prototype 对历史 `wait_agent` tool call 的摘要展示继续走现有 typed wait/collab tool 映射，不作为 raw marker 兼容路径扩展。

## 风险

- pending input 的立即唤醒必须使用 canonical typed 数据源，不能为了匹配 child completion 去解析 assistant 文本，否则会破坏 `ResponseItem -> ThreadItem` 约束。
- mailbox 检查只能 peek，不能 drain；否则后续 model-visible history 会丢失 IAC。
- hard cap 较长，测试需要使用可配置的短 timeout 覆盖超时路径，避免真实等待。

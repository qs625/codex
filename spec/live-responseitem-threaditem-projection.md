# Live ResponseItem 到 ThreadItem 投影修复

## 任务 brief

用户反馈 app-server live 场景最后一条消息不显示，重启后通过 history 重建才显示。成功标准是：

- live `item/started` / `item/completed` 和 `thread/history` 重建都以 typed `ThreadItem` 为展示 payload。
- 最终 assistant message 在 live 流中可见，不需要等重启后从 rollout history 重建。
- `function_call`、`event_command_subscribe`、schedule subscribe/unsubscribe、event-command event、collab / inter-agent / child-completion 继续复用 shared `ResponseItem -> ThreadItem` projector。
- 非 TUI 展示/history 路径不新增 raw response item 展示分支，不从 assistant marker JSON 或 message marker 文本反解展示项。

非目标是不修改 TUI，不新增 app-server v1 API surface，不改变 provider wire/model input 的 marker formatting 边界。

## 探索结论

app-server live 路径中，listener 会先把 core `EventMsg` 写入 `ThreadState.current_turn_history`，再调用 `apply_bespoke_event_handling` 发 v2 通知。legacy `ItemStarted` / `ItemCompleted` 已经在 app-server protocol 边界转换为 typed `ThreadItem`。

history 重建路径由 `ThreadHistoryBuilder` 回放 rollout。它会把 persisted `RolloutItem::ResponseItem(ResponseItem::Message { role: "assistant", ... })` 投影成 `ThreadItem::AgentMessage`，因此重启或 `thread/read includeTurns` 后能看到最后一条 assistant message。

live/history 路径有两个相关缺口：

- `EventMsg::RawResponseItem` 在 app-server live apply 层只保留 hook prompt 辅助路径，不作为普通 display item 发射。最终 assistant message 的 live 展示应来自 semantic lifecycle `ItemCompleted(TurnItem::AgentMessage)`，history/recovery 则由 typed `ResponseItem::Message` canonicalize。
- running-thread resume / live read 使用的 in-memory `ThreadHistoryBuilder` 会消费 core lifecycle event，但 `handle_item_completed` 原来忽略 `TurnItem::AgentMessage`。因此运行中重连或 `thread/read includeTurns` 合并 active turn 时，也可能缺少最后 assistant message；重启后从 rollout `ResponseItem::Message` 重建则能显示。

## 技术设计

最小修复是在 `codex-rs/app-server-protocol/src/protocol/response_item_projection.rs` 收敛 structured item projector 和 legacy raw structured message 过滤：

- `EventCommandEvent`、`EventDrivenTool`、已知 `InterAgentCommunication` 继续复用既有 `project_structured_response_item`。
- `FunctionCall` / `FunctionCallOutput` 保持既有 live helper：start 时发 `item/started`，output 时发 completed tool item。
- user hook prompt 仍保留当前 `RawResponseItem` hook prompt helper。

app-server live apply 层不把普通 `RawResponseItem` 直接投影为 display `ThreadItem`，避免和 semantic lifecycle 双发；history builder 在 recovery/read 路径把 typed `ResponseItem::Message` canonicalize 为 `ThreadItem::AgentMessage`，并过滤 legacy marker / inter-agent JSON envelope。

`ThreadHistoryBuilder::handle_item_completed` 对 typed `TurnItem::AgentMessage` 做 `ThreadItem::from` 并按 `turn_id` upsert，使 active/current reducer 与 persisted history reducer 使用同一 typed display item。若同一 assistant message 随后又以 `RawResponseItem(Message)` 进入 history builder，按同文本/phase 消费 pending response，避免 final answer 重复展示。

后续发现 root-worker renderer 在切换 thread 时还存在独立的展示一致性问题：

- root-worker renderer 的完整 turn/item cache 以 readOnce 为初始化边界。客户端启动后每个 thread 默认未初始化；未初始化 thread 收到 live `turn/*`、`item/*`、`item/agentMessage/delta` 或 child completion 时，只允许更新 thread list 级别的状态/摘要，不写入会话展示用的 turns/items cache。
- 用户第一次查看某个 thread 时，调用一次 `thread/read includeTurns` 建立完整历史基线，并把该 thread 标记为 initialized。这个 snapshot 是该 thread 完整 item cache 的唯一初始化来源。
- `thread/read` 的 in-flight token 必须按 thread 维度管理；A thread 的 read 在途时切到 B 再切回 A，B 的 read 不应让 A 的 read 结果变 stale。只有同一 thread 的更新请求可以 supersede 旧请求。
- 已初始化 thread 后续无论是否 selected，都只消费 typed v2 live `ThreadItem` 增量更新 cache；切换回来不再触发 `thread/read includeTurns`，也不允许 snapshot/history rebuild 覆盖、重排或 merge 已接收的 live items。
- child completion 按 typed item 的目标 thread id 更新 initialized thread cache；如果目标 thread 未初始化，则不创建 synthetic turn 或 mixed `agentMessage + childCompletion` cache，避免首次 read 前的部分 live item 和后续 snapshot 互相消费。
- `thread/read` 只保留给 cold start、本地缺失线程、或显式恢复路径；这些路径进入本地状态前必须继续 canonicalize 为 typed `ThreadItem`，不得从 raw marker/message envelope 反解展示项。

legacy raw inter-agent 文本只作为旧兼容输入过滤，不作为展示来源：

- canonical typed collab message 使用 `operation: "sendMessage"`；`send_message` 仅作为 legacy raw assistant JSON envelope 的旧拼写被过滤。
- 完整 raw JSON envelope、XML marker 以及它们的流式分片都应在 root-worker `appendAgentDelta` 层吞掉，避免短暂显示 marker 或 JSON。普通 assistant 文本和普通 JSON 分片必须继续显示。

## 测试设计

新增 app-server 单元测试覆盖：

- `RawResponseItem` 普通 assistant display 在 live apply 层不直接发 completed display item，hook prompt 仍通过专门 helper 发 completed item。
- structured typed response item 在 history/recovery 中通过 shared projector 重建 `ThreadItem`，确保 event-command / collab 类 item 不走 raw 展示分支。

新增 app-server-protocol 单元测试覆盖：

- `EventMsg::ItemCompleted(TurnItem::AgentMessage)` 会进入 active/history builder 的 turn items。
- `EventMsg::ItemCompleted(TurnItem::AgentMessage)` 后接同 id/text 的 `RawResponseItem(Message)` 不重复展示。

复用已有 `event_command_call_notifications_emit_started_then_completed` 和 hook prompt 测试覆盖 function call 与 hook prompt 的 live helper。

新增 root-worker 测试覆盖：

- thread selection policy 对未 initialized 的本地 live thread 仍执行首次 `thread/read`，对 initialized live thread 只补 subscribe、不再 read。
- initialized thread 收到 child completion 后保留既有 assistant message，并正确追加/更新 completion 展示项。
- uninitialized thread 收到 child completion/live item 不创建可展示 mixed turn cache，首次查看仍由 `thread/read` 建立基线。
- cold/missing thread 仍保留 `thread/read`，用于首次加载或恢复。
- snapshot、compact replacement history、conversation reducer 过滤 legacy raw `sendMessage` / `send_message` envelope，但保留 typed `CollabAgentMessage(sendMessage)`。
- `appendAgentDelta` 过滤 XML marker 分片、legacy raw JSON envelope 分片，并验证普通 JSON 分片不会被误吞。

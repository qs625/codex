# Live ResponseItem 到 ThreadItem 投影修复

> 迁移说明：本文记录早期 live/history 投影修复背景。新的架构目标已调整为
> `ResponseItem = model/context source`、`EventMsg = runtime/UI display source`、
> `ThreadItem = client display projection`。后续新增展示语义以
> [EventMsg 作为 ThreadItem 展示源](eventmsg-threaditem-display-source.md) 为准；
> 本文中的 `ResponseItem -> ThreadItem` 主路径表述只代表历史阶段和 legacy 兼容。

## 任务 brief

用户反馈 app-server live 场景最后一条消息不显示，重启后通过 history 重建才显示。成功标准是：

- live `item/started` / `item/completed` 和 `thread/history` 重建都以 typed `ThreadItem` 为展示 payload。
- 最终 assistant message 在 live 流中可见，不需要等重启后从 rollout history 重建。
- `function_call`、`event_command_subscribe`、schedule subscribe/unsubscribe、event-command event、collab / inter-agent / child-completion 在迁移期继续复用 legacy structured projector；新增展示语义应迁向 `EventMsg -> ThreadItem` projector。
- `event_command_subscribe` 工具调用本身在 live/app-server/root-worker 中产生 typed `ThreadItem::EventCommandCall` started/completed；后续 event output/completion 产生 typed `ThreadItem::EventCommandEvent`，两者都不能只以 raw JSON/message 出现。
- 非 TUI 展示/history 路径不新增 raw response item 展示分支，不从 assistant marker JSON 或 message marker 文本反解展示项。

非目标是不修改 TUI，不新增 app-server v1 API surface，不改变 provider wire/model input 的 marker formatting 边界。

## 探索结论

app-server live 路径中，listener 会先把 core `EventMsg` 写入 `ThreadState.current_turn_history`，再调用 `apply_bespoke_event_handling` 发 v2 通知。legacy `ItemStarted` / `ItemCompleted` 已经在 app-server protocol 边界转换为 typed `ThreadItem`。

history 重建路径由 `ThreadHistoryBuilder` 回放 rollout。它会把 persisted `RolloutItem::ResponseItem(ResponseItem::Message { role: "assistant", ... })` 投影成 `ThreadItem::AgentMessage`，因此重启或 `thread/read includeTurns` 后能看到最后一条 assistant message。

live/history 路径有两个相关缺口：

- `EventMsg::RawResponseItem` 在 app-server live apply 层只保留旧事件兼容处理，不作为普通 display item 发射。最终 assistant message 的 live 展示应来自 semantic lifecycle `ItemCompleted(TurnItem::AgentMessage)`，history/recovery 则由 typed `ResponseItem::Message` canonicalize。
- running-thread resume / live read 使用的 in-memory `ThreadHistoryBuilder` 会消费 core lifecycle event，但 `handle_item_completed` 原来忽略 `TurnItem::AgentMessage`。因此运行中重连或 `thread/read includeTurns` 合并 active turn 时，也可能缺少最后 assistant message；重启后从 rollout `ResponseItem::Message` 重建则能显示。

## 技术设计

### TurnItem 迁移阶段

早期扩展目标曾是让 `ResponseItem` 成为 core live event、history 和 UI lifecycle 的 canonical state。当前 EventMsg 重构后，该目标已被替换：`ResponseItem` 只负责模型/context/provider 侧事实，`EventMsg` 负责 runtime/UI display source，app-server v2 display payload 通过 `EventMsg -> ThreadItem` projector 生成。一次性删除 `TurnItem` 仍不适合作为本阶段闭环：`TurnItemContributor` extension API、stream finalization 的 `FinalizedTurnItemFacts`、TTFM metrics、legacy event 生成、旧 rollout replay，以及 `UserMessage` 的 `text_elements` 保留都仍直接依赖 `TurnItem`。

本阶段采用分阶段迁移：

- 第一阶段：新增 typed `ResponseItemCompleted` lifecycle 事件，app-server v2 对该事件统一复用 shared projector。没有 `TurnItem` 变体的 typed display completion 优先走 `ResponseItem` lifecycle；旧 `ItemStarted` / `ItemCompleted(TurnItem)` 仅保留为 legacy rollout / 旧 emit 点兼容适配，不再作为新增展示语义的扩展方向。
- 第二阶段：补齐 typed started lifecycle，扩展 `response_item_projection` 覆盖现有 14 个 `TurnItem` variant 对应的 `ResponseItem` 表达，并逐个把 core emit 点从 `emit_turn_item_*` 迁到 `emit_response_item_*`。每个迁移都必须明确 provider/model-visible 处理由 request builder 决定，display/history 不解析 raw marker 或 assistant JSON。
- 第三阶段：迁移 extension contributor、TTFM 和 stream finalization 的内部事实结构，删除 `parse_turn_item` 和 `TurnItem -> ThreadItem` app-server adapter；只保留读取旧 rollout 的兼容转换，直到旧数据兼容窗口结束。

第一阶段完成后，`CommandWait`、`CommandWriteStdin`、`CommandExecutionNotification` 等没有 `TurnItem` 变体的 display item 能走 typed completed lifecycle；后续新增 completed display item 不需要也不应该扩展 `TurnItem`。

最小修复是在 `codex-rs/app-server-protocol/src/protocol/response_item_projection.rs` 收敛 structured item projector 和 legacy raw structured message 过滤：

- `EventCommandEvent`、`EventDrivenTool`、已知 `InterAgentCommunication` 继续复用既有 `project_structured_response_item`。
- `event_command_subscribe` 的 `FunctionCall` / `FunctionCallOutput` 继续通过 `project_tool_response_item` 投影为 `ThreadItem::EventCommandCall`，表示订阅工具调用本身的 started/completed lifecycle。
- `FunctionCall` / `FunctionCallOutput` 保持既有 live helper：start 时发 `item/started`，output 时发 completed tool item。
- user hook prompt 由 `record_response_item_and_emit_turn_item` 这类显式 lifecycle 记录函数发出 typed `item/completed`，不再依赖 `record_conversation_items` 的 raw live fanout。

core 的 `record_conversation_items` 只负责写入 in-memory history、persist `RolloutItem::ResponseItem` 并刷新 context usage；需要 live 展示的路径必须显式发 typed lifecycle。app-server live apply 层不把普通 `RawResponseItem` 直接投影为 display `ThreadItem`，避免和 semantic lifecycle 双发；history builder 在 recovery/read 路径把 typed `ResponseItem::Message` canonicalize 为 `ThreadItem::AgentMessage`。root-worker renderer 不再解析或过滤 legacy marker / inter-agent JSON envelope，避免展示层根据文本内容丢 typed item。

业务入口不得为了“即时展示”直接 `send_event_raw(ItemCompleted(...))` 发 conversation display item。child completion 收到时只负责写入 mailbox、清理 direct-child outstanding 屏障并按 `trigger_turn` 唤醒 pending work；真正的 collab/child-completion 展示项在迁移期由 pending input 录入阶段生成 `ResponseItem::InterAgentCommunication`，再通过 `record_model_items_and_emit_display_events` 双写 model item 和 display event。后续新增 collab 展示语义应使用 dedicated `EventMsg` variant。这样 live app-server 不会同时收到旧 `ItemCompleted(TurnItem::CollabAgentMessage)` 和 typed 录入后的 completed display item。

`CommandWait`、`CommandWriteStdin`、`CommandExecutionNotification` 这类工具交互历史项在迁移期仍保留 typed `ResponseItem`，但没有对应的 legacy `TurnItem` 变体。live completed 路径不能为了展示继续扩展 `TurnItem`，而应在 core 发出 display-capable `EventMsg`；当前兼容路径使用 `EventMsg::ResponseItemStarted/Completed(ResponseItem)`，由 app-server v2 的 `event_item_projection` 投影为 typed `ThreadItem`。root-worker 只消费 typed `ThreadItem::CommandWait` 等 payload，不从 raw marker、assistant text 或 legacy envelope 反解。

`record_model_items_and_emit_display_events` 在写 in-memory history 和发 live completed 前，会为缺失 id 的 structured model/display item 生成同一个本地 display id，live root-worker 不会在同一 turn 多个 command wait/stdin 项之间互相覆盖。旧 id-less lifecycle event 在 history replay 中仍按兼容规则跳过，避免重启回放时把同一 command wait/stdin 双写；带 id 的 `ResponseItemStarted/Completed` 会通过 `EventMsg -> ThreadItem` adapter replay。

`RawResponseItem` 协议分支暂时保留为旧 rollout / 旧 client 兼容输入，但新的 runtime 不再通过 `record_conversation_items` 广播 raw response item。provider 请求侧仍可在最后一步把 typed `ResponseItem` formatting 成 provider-visible marker message；该 formatting 产物不得写回 history、rollout 或 display。

`ThreadHistoryBuilder::handle_item_completed` 对 typed `TurnItem::AgentMessage` 做 `ThreadItem::from` 并按 `turn_id` upsert，使 active/current reducer 与 persisted history reducer 使用同一 typed display item。若同一 assistant message 随后又以 `RawResponseItem(Message)` 进入 history builder，按同文本/phase 消费 pending response，避免 final answer 重复展示。

后续发现 root-worker renderer 在切换 thread 时还存在独立的展示一致性问题：

- root-worker renderer 的完整 turn/item cache 以 readOnce 为初始化边界。客户端启动后每个 thread 默认未初始化；未初始化 thread 收到 live `turn/*`、`item/*`、`item/agentMessage/delta` 或 child completion 时，只允许更新 thread list 级别的状态/摘要，不写入会话展示用的 turns/items cache。
- 用户第一次查看某个 thread 时，调用一次 `thread/read includeTurns` 建立完整历史基线，并把该 thread 标记为 initialized。这个 snapshot 是该 thread 完整 item cache 的唯一初始化来源。
- `thread/read` 的 in-flight token 必须按 thread 维度管理；A thread 的 read 在途时切到 B 再切回 A，B 的 read 不应让 A 的 read 结果变 stale。只有同一 thread 的更新请求可以 supersede 旧请求。
- 已初始化 thread 后续无论是否 selected，都只消费 typed v2 live `ThreadItem` 增量更新 cache；切换回来不再触发 `thread/read includeTurns`，也不允许 snapshot/history rebuild 覆盖、重排或 merge 已接收的 live items。
- 已初始化 thread 的 `turn/started` / `turn/completed` 只更新 turn lifecycle 元数据，例如 status、started/completed/duration/error/itemsView；不得把通知里的 `turn.items` 当作新 snapshot 覆盖本地 items。conversation item 内容只由 `item/started`、`item/completed`、agent delta 等 typed live item 增量写入。
- `thread/resume(excludeTurns=true)` 只负责建立 live 订阅和刷新 runtime/metadata；对 renderer 已有 thread 不写入空 turns snapshot，避免切换 thread 或补订阅时清理已经显示的 live-only child completion / subagent status。
- child completion 按 typed item 的目标 thread id 更新 initialized thread cache；如果目标 thread 未初始化，则不创建 synthetic turn 或 mixed `agentMessage + childCompletion` cache，避免首次 read 前的部分 live item 和后续 snapshot 互相消费。
- `thread/read` 只保留给 cold start、本地缺失线程、或显式恢复路径；这些路径进入本地状态前必须继续 canonicalize 为 typed `ThreadItem`，不得从 raw marker/message envelope 反解展示项。
- 连续普通 agent message 在 `ConversationCell` 层合并后，`MessageRow` 必须渲染为单个 agent bubble；cell 内每个 entry 作为 bubble 内的 message segment 保留文本和附件，避免视觉上仍像多个 assistant cell。

legacy raw inter-agent 文本不作为结构化展示来源：

- canonical typed collab message 使用 `operation: "sendMessage"`；`send_message` 作为 legacy raw assistant JSON envelope 的旧拼写出现在历史文本时保留为 literal message，不在 renderer 反解或过滤。
- 完整 raw JSON envelope、XML marker 以及它们的流式分片如果到达 root-worker `appendAgentDelta`，按普通 assistant 文本保留；结构化 child-completion/subagent/event-command 展示只能来自 typed `ThreadItem`。

## 测试设计

新增 app-server 单元测试覆盖：

- `event_command_subscribe` 工具调用发出 typed `EventCommandCall` started/completed，completed payload 保留 `subscription_id`、`command`、`cwd`、`label` 和 output。
- `RawResponseItem` 普通 assistant display 在 live apply 层不直接发 completed display item，hook prompt 通过显式 response item helper 发 completed item。
- structured typed response item 在 history/recovery 中通过 shared projector 重建 `ThreadItem`，确保 event-command / collab 类 item 不走 raw 展示分支。

新增 app-server-protocol 单元测试覆盖：

- `EventMsg::ItemCompleted(TurnItem::AgentMessage)` 会进入 active/history builder 的 turn items。
- `EventMsg::ItemCompleted(TurnItem::AgentMessage)` 后接同 id/text 的 `RawResponseItem(Message)` 不重复展示。
- `EventMsg::ResponseItemCompleted(ResponseItem::CommandWait)` 会通过 shared projector 映射为 v2 `ItemCompletedNotification`，payload item 是 `ThreadItem::CommandWait`，并保留 command id、状态、通知、exit code、耗时和创建时间。
- `ThreadHistoryBuilder` 回放 `ResponseItem(CommandWait { id: None })` 后再回放 `ResponseItemCompleted(CommandWait { id: None })` 时只保留一个 `ThreadItem::CommandWait`，证明 completed event 不会在 cold/history rebuild 中制造重复展示。

新增 core 单元测试覆盖：

- `record_model_items_and_emit_display_events` 对缺失 id 的 `ResponseItem::CommandWait` 生成 display id 并发出 `EventMsg::ResponseItemCompleted`，证明迁移期 structured item 不再因无法转换为 legacy `TurnItem` 被 live completed 路径静默丢弃。
- `inter_agent_communication(ChildCompletion)` 收到 mailbox 消息时不直接发 live collab `ItemCompleted`；同一个 child completion 经 pending input typed 录入后只发出一条 collab completed item，防止 live app-server/root-worker 看到两条 child completion 展示项。

复用已有 `event_command_call_notifications_emit_started_then_completed` 和 hook prompt 测试覆盖 function call 与 hook prompt 的 live helper。

新增 root-worker 测试覆盖：

- thread selection policy 对未 initialized 的本地 live thread 仍执行首次 `thread/read`，对 initialized live thread 只补 subscribe、不再 read。
- initialized thread 收到 child completion 后保留既有 assistant message，并正确追加/更新 completion 展示项。
- initialized thread 已经显示 child completion 后，后续缺少该 item 的 `turn/completed` lifecycle 通知不会删除本地 completion。
- `thread/resume(excludeTurns=true)` 补订阅返回的 metadata snapshot 不会覆盖 renderer 已有 turns。
- 连续 agent entries 在同一个 message cell 内只生成一个 `.message-bubble`，内部按 segment 呈现。
- uninitialized thread 收到 child completion/live item 不创建可展示 mixed turn cache，首次查看仍由 `thread/read` 建立基线。
- cold/missing thread 仍保留 `thread/read`，用于首次加载或恢复。
- snapshot、compact replacement history、conversation reducer 不从 legacy raw `sendMessage` / `send_message` envelope 反解展示项；typed `CollabAgentMessage(sendMessage)` 保持为 canonical display item。
- `appendAgentDelta` 保留 XML marker 分片、legacy raw JSON envelope 分片和普通 JSON 分片的 literal text，验证 renderer 不按 raw 文本丢 item。

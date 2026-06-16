# EventMsg 作为 ThreadItem 展示源

## 任务 brief

本重构把对话展示链路收敛为三层：

- `ResponseItem`：模型交互、context manager/provider history、compact、guardian 和模型可见工具输出。
- `EventMsg`：runtime event log 和 UI display source。
- `ThreadItem`：app-server v2、root-worker 和 SDK 客户端消费的展示投影。

成功标准：

- provider/model 请求继续直接消费 context 中的 `ResponseItem`，不做 `EventMsg -> ResponseItem` 反向重建。
- live `item/started` / `item/completed` 和 thread/read replay 共享 `EventMsg -> ThreadItem` adapter。
- 同时需要模型可见和 UI 可见的业务动作通过 helper 双写：记录 model-visible `ResponseItem`，并 emit display-capable `EventMsg`。
- 新增 display-only 语义优先新增专用 `EventMsg` variant，不再新增 display-only `ResponseItem` variant。
- legacy `ResponseItem -> ThreadItem`、`TurnItem -> ThreadItem`、`RawResponseItem` 只保留为旧 rollout/history 兼容边界。

非目标：

- 不改变 provider wire formatting 的 marker 包装；marker 只能作为单向 provider formatting。
- 不新增 app-server v1 API surface。
- 不一次性删除所有 legacy rollout 兼容代码。

## 当前主线事实

### ResponseItem -> ThreadItem

`codex-rs/app-server-protocol/src/protocol/response_item_projection.rs` 仍是 legacy 兼容 projector，覆盖：

- `CommandWait`
- `CommandWriteStdin`
- `CommandExecutionNotification`
- `WorkflowRunProgress`
- `EventCommandEvent`
- `EventDrivenTool`
- `ThreadGoalUpdate`
- `InterAgentCommunication`

这些 `ResponseItem` 变体暂时仍保留，因为当前 context/history/provider 边界还需要它们表达模型侧事实；但它们不再是新展示语义的扩展点。

### EventMsg -> ThreadItem

`codex-rs/app-server-protocol/src/protocol/event_item_projection.rs` 是共享 projection boundary：

- `EventMsg::ItemStarted(TurnItem)` / `ItemCompleted(TurnItem)`：legacy lifecycle 兼容。
- `EventMsg::ResponseItemStarted(ResponseItem)` / `ResponseItemCompleted(ResponseItem)`：迁移期兼容，内部复用 structured `ResponseItem` projector。
- 输出 `ProjectedEventItem::{Started, Completed}`，由 live notification 和 thread history reducer 统一投影为 `ThreadItem`。

`event_mapping.rs` 先调用 `project_event_msg_item`，再生成 v2 `ItemStartedNotification` / `ItemCompletedNotification`。`ThreadHistoryBuilder` 对带 display id 的 `ResponseItemStarted/Completed` 也复用同一 adapter，并按 `ThreadItem.id` upsert；旧 id-less lifecycle event 在 history replay 中继续跳过，避免和 persisted `ResponseItem` 形成重复展示。

### 双写 helper

`Session::record_model_items_and_emit_display_events` 是模型上下文与 UI 事件的一致性入口：

- 先写入 in-memory history 和 rollout `ResponseItem`。
- 再 emit display-capable `EventMsg`。
- 对缺失 id 的 structured display model item 补稳定 item id，保证 live/replay 可按 `ThreadItem.id` 合并。

## 迁移路线

1. command/wait family
   - `command_wait` started/completed 已通过 `ResponseItemStarted/Completed` 进入 EventMsg adapter。
   - `command_write_stdin`、command execution notification 继续走 dual-write helper，后续应迁为专用 display `EventMsg` variant。
   - raw tool output JSON 不得作为 UI display source。

2. collaboration/child completion
   - `InterAgentCommunication` 当前仍作为 model item 记录，并由 EventMsg lifecycle 进入 display projector。
   - 后续把 inter-agent display 和 child completion status 更新迁成 dedicated `EventMsg` variant。
   - legacy `ResponseItem::InterAgentCommunication` 只保留旧 history/provider 兼容。

3. goal/workflow/event-command
   - `ThreadGoalUpdate`、`WorkflowRunProgress`、`EventCommandEvent`、`EventDrivenTool` 当前保留为迁移期 structured model items。
   - 新增展示能力必须先设计 EventMsg variant 和 projector，不再扩展 ResponseItem 作为纯 UI 容器。

4. legacy cleanup
   - 隔离 live `TurnItem -> ThreadItem` 到旧 lifecycle adapter。
   - 保留 `RawResponseItem` 只用于旧 rollout/history rebuild 和 hook prompt 兼容。
   - root-worker 只能消费 typed `ThreadItem`，不得从 raw marker、assistant JSON、legacy envelope 或 raw FunctionCallOutput 反解 display item。

## 验证策略

每个迁移 family 至少覆盖：

- protocol projector 测试：`EventMsg -> ProjectedEventItem -> ThreadItem` payload 完整。
- live app-server 测试：v2 `item/started` / `item/completed` 使用 typed `ThreadItem`。
- thread/read replay 测试：persisted `EventMsg` 与 legacy `ResponseItem` 不重复展示。
- provider/model 测试：模型上下文仍从 `ResponseItem` 获取 request input，不从 `EventMsg` 反推。

Rust/Cargo 验证必须由固定 tester `/root/my_codex_pm/rust_cargo_tester` 串行执行。

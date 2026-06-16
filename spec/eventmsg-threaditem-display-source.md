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
- `ResponseItem -> ThreadItem`、公开 `TurnItem -> ThreadItem` adapter、`RawResponseItem` 展示回放路径已删除；需要 UI 展示的事实必须写成 display-capable `EventMsg`。

非目标：

- 不改变 provider wire formatting 的 marker 包装；marker 只能作为单向 provider formatting。
- 不新增 app-server v1 API surface。
- 不保留旧 rollout/history 的展示兼容；旧文件中只有 `ResponseItem` / `RawResponseItem` 的内容不会重建为 UI 展示。

## 当前主线事实

### ResponseItem

`ResponseItem` 只用于模型交互、context manager/provider history、compact、guardian 和模型可见工具输出。app-server thread display 不再从以下来源投影 `ThreadItem`：

- `RolloutItem::ResponseItem`
- `EventMsg::RawResponseItem`
- `EventMsg::ResponseItemStarted`
- `EventMsg::ResponseItemCompleted`
- live `ResponseItem::FunctionCall` / `FunctionCallOutput`

这些 `ResponseItem` 变体仍可存在于模型上下文和 provider history 中，但它们不是展示源，也不是新增展示语义的扩展点。

### EventMsg -> ThreadItem

`codex-rs/app-server-protocol/src/protocol/event_item_projection.rs` 是共享 projection boundary：

- `EventMsg::ItemStarted(TurnItem)` / `ItemCompleted(TurnItem)`：作为 EventMsg lifecycle payload 由 `event_item_projection.rs` 显式投影；不再保留公开 `TurnItem -> ThreadItem` adapter。
- `EventMsg::ResponseItemStarted(ResponseItem)` / `ResponseItemCompleted(ResponseItem)`：不产生 `ThreadItem`，仅作为非展示 runtime/model 历史事件保留。
- `EventMsg::CommandWaitStarted` / `CommandWaitCompleted` / `CommandWriteStdinCompleted` / `CommandExecutionNotificationCompleted`：command session 展示主路径。
- `EventMsg::WorkflowRunProgressCompleted` / `ThreadGoalUpdateCompleted` / `EventCommandEventCompleted` / `EventDrivenToolCompleted` / `InterAgentCommunicationCompleted`：workflow、goal、event command、event-driven tool 和协作展示主路径。
- 输出 `ProjectedEventItem::{Started, Completed}`，由 live notification 和 thread history reducer 统一投影为 `ThreadItem`。

`event_mapping.rs` 先调用 `project_event_msg_item`，再生成 v2 `ItemStartedNotification` / `ItemCompletedNotification`。`ThreadHistoryBuilder` 只从 persisted display-capable `EventMsg` 重建 `ThreadItem`；旧 rollout 没有 dedicated display event 时不会重建可读 UI 历史。

### 双写 helper

`Session::record_model_items_and_emit_display_events` 是模型上下文与 UI 事件的一致性入口：

- 先写入 in-memory history 和 rollout `ResponseItem`。
- 再 emit display-capable `EventMsg`。
- 对缺失 id 的 structured display model item 补稳定 item id，保证 live/replay 可按 `ThreadItem.id` 合并。

## 迁移路线

1. command/wait family
   - `command_wait` started/completed 通过 `CommandWaitStarted` / `CommandWaitCompleted` 进入 EventMsg adapter。
   - `command_write_stdin`、command execution notification 通过 `CommandWriteStdinCompleted` / `CommandExecutionNotificationCompleted` 进入 EventMsg adapter。
- `ResponseItem::CommandWait`、`CommandWriteStdin` 和 `CommandExecutionNotification` 只在模型可见时保留，不参与 history display replay。
- `ResponseItem::FunctionCall` / `FunctionCallOutput` 不再作为 live tool-call display source；需要展示工具调用时必须发 dedicated display `EventMsg`。
   - raw tool output JSON 不得作为 UI display source。

2. collaboration/child completion
   - `InterAgentCommunicationCompleted` 是 inter-agent display 的主路径。
   - `ResponseItem::InterAgentCommunication` 只保留为模型/provider history，不参与 display replay。

3. goal/workflow/event-command
   - `ThreadGoalUpdateCompleted`、`WorkflowRunProgressCompleted`、`EventCommandEventCompleted`、`EventDrivenToolCompleted` 是对应展示主路径。
   - `ResponseItem::ThreadGoalUpdate`、`WorkflowRunProgress`、`EventCommandEvent`、`EventDrivenTool` 保留为模型历史，不参与 display replay。

4. legacy cleanup
   - 删除公开 `TurnItem -> ThreadItem` adapter，只允许 `event_item_projection.rs` 在处理 `EventMsg::ItemStarted/Completed` 时显式转换。
   - `RawResponseItem` 不再用于 live 或 thread/read display。
   - root-worker 只能消费 typed `ThreadItem`，不得从 raw marker、assistant JSON、legacy envelope 或 raw FunctionCallOutput 反解 display item。

## 验证策略

每个迁移 family 至少覆盖：

- protocol projector 测试：`EventMsg -> ProjectedEventItem -> ThreadItem` payload 完整。
- live app-server 测试：v2 `item/started` / `item/completed` 使用 typed `ThreadItem`。
- thread/read replay 测试：persisted display-capable `EventMsg` 能重建展示，`ResponseItem` / `RawResponseItem` 不产生展示项。
- provider/model 测试：模型上下文仍从 `ResponseItem` 获取 request input，不从 `EventMsg` 反推。

Rust/Cargo 验证必须由固定 tester `/root/my_codex_pm/rust_cargo_tester` 串行执行。

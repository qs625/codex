# 修复 mailbox raw envelope 客户端泄漏

## 任务 brief

- 用户：使用多 agent 协作能力的 Codex 用户，以及消费 core/app-server typed events 的客户端。
- 缺陷：mailbox / inter-agent communication 在进入模型 pending input 链路时过早转换成 assistant `output_text` JSON，导致内部协作协议和普通 assistant raw string 的边界不清晰。
- 成功标准：
- mailbox / async thread input 在 core pending input、requeue、hook inspect 链路中保持结构化类型。
  - user message / tool call output 继续按普通 `ResponseInputItem` 处理。
  - inter-agent communication 不作为普通 assistant streaming delta 语义进入客户端；客户端只消费普通 assistant raw string 的 `AgentMessageContentDelta` 和 completed 阶段的结构化 collab item。
  - provider 请求 wire shape 保持兼容，直到最终请求格式化边界才把 typed inter-agent communication 转换成模型可接受的 response item。
- 非目标：
  - 不在 app-server 或客户端按 raw JSON shape 做兜底过滤。
  - 不删除旧 assistant JSON 的历史解析兼容能力。
  - 不改变模型输出 raw assistant text 的 streaming 行为。

## 技术设计

修复点放在 core 输入链路，而不是 response stream、app-server 或客户端。

新增 core 内部 enum `PendingInputItem`：

- `ResponseInput(ResponseInputItem)`：用户输入、tool output、idle queued response item 等普通输入。
- `ResponseItem(ResponseItem)`：event command / event-driven tool 等已经是 thread typed item 的异步输入。
- `InterAgentCommunication(InterAgentCommunication)`：mailbox 投递的 agent 间结构化消息。

链路调整：

- `Session::get_pending_input()` drain mailbox 时返回 typed `PendingInputItem`，不再调用 `InterAgentCommunication::to_response_input_item()`。
- `TurnState` 持有 `Vec<PendingInputItem>`，requeue / abort preservation / active turn injection 都保留 typed input。
- `inspect_pending_input()` 只对 `PendingInputItem::ResponseInput` 中的 user message 执行 user-prompt-submit hook；inter-agent communication 直接作为结构化 pending input 接受。
- `record_pending_input()` 在记录会话历史时通过 `PendingInputItem::into_response_item()` 统一转换到 typed `ResponseItem`。
- `ResponseItem` 增加 `EventCommandEvent` / `EventDrivenTool` / `InterAgentCommunication` typed variants；provider 请求格式化时才降级为现有 wire message。
- `CodexThread::append_message()` 不再走独立 `inject_response_items` / `queue_response_items_for_next_turn` 分支，而是把 typed async input 入同一个 mailbox；非 user 异步输入默认 `trigger_turn = true`，只有 `InterAgentCommunication.trigger_turn = false` 的 queue-only agent mail 不触发 turn。

这样 mailbox 输入和普通 assistant raw text 的分界在 core 内部是类型系统表达的，不再依赖 raw JSON parse 或 response stream 的暂存状态机。

## 兼容性

当前 provider/history 仍以 `ResponseItem` 表示可重放上下文，因此 inter-agent communication 在最终记录和请求格式化边界仍会转换为 assistant commentary message。该转换集中在 `PendingInputItem::into_response_item()`，避免在 pending input 链路中扩散 raw string。

旧 rollout / 历史中已经存在的 assistant JSON 仍由既有 `InterAgentCommunication::from_message_content()` 和 `parse_turn_item()` 解析，保证 completed collab item 的兼容行为不变。

## 其他输入源核对

- `spawn_agent` / `send_message` / `followup_task` / child completion / subagent notification 都以 `InterAgentCommunication` 为协议对象，目标 thread 通过 `Op::InterAgentCommunication` 进入 `Session::enqueue_mailbox_communication()`，因此受本次 `PendingInputItem::InterAgentCommunication` 修复覆盖。
- child completion 还会额外发 live `TurnItem::CollabAgentMessage` completed item，这是客户端 typed item，不走普通 assistant delta。
- `event_command_subscribe` 后续触发事件由 `file-subscription` runtime 生成 `EventCommandEvent`，现在通过 typed `ResponseItem::EventCommandEvent` 进入同一个 async mailbox；旧 `<event_command>...</event_command>` parser 只保留历史兼容。
- `EventDrivenToolTrigger` 与 event command 类似，现在通过 typed `ResponseItem::EventDrivenTool` 进入 async mailbox；旧 marker parser 只保留历史兼容。

## 风险

- rollout / history 会开始记录 typed `ResponseItem` variants；旧 marker message 仍可解析，兼容已有历史。
- 如果未来 provider 支持原生 typed inter-agent input，可以把 `PendingInputItem::into_response_item()` 的降级逻辑替换到 provider adapter 中。
- 任意模型主动输出看起来像 inter-agent JSON 的普通 assistant 文本仍会按模型输出链路处理；本修复不把模型输出当 mailbox 输入解析。
- provider wire 仍使用现有 marked text / assistant commentary message 兼容 Responses API；typed 到 wire 的降级集中在 prompt formatting 边界。

## 验证计划

- session 单测：mailbox 在 answer boundary 后转入下一 turn 时，`get_pending_input()` 返回 `PendingInputItem::InterAgentCommunication`。
- session 单测：steered input 和 mailbox 同时 pending 时，user input 是 `PendingInputItem::ResponseInput`，mailbox 是 `PendingInputItem::InterAgentCommunication`，顺序保持不变。
- session 单测：tool call reopen mailbox delivery 时，mailbox pending 类型保持结构化。
- pending input 集成测试：最终 `/responses` 请求仍保持现有 wire payload snapshot，确保 provider 兼容不变。

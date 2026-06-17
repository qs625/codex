# Goal post-turn state

## 背景

Goal runtime 会在 active turn 清理后尝试自动启动下一轮 continuation，并注入
`<goal_context>`。child completion 则在 subagent 进入 final status 后通知 parent。两条链路都发生在
turn complete 之后，但原逻辑分别判断：Goal continuation 只看本 thread 是否没有 active turn /
queued input / trigger-turn mailbox，child completion 只看 child runtime 是否仍 active。这样会出现两类
问题：

- Goal continuation、child completion、command/event-command wait 的优先级不清晰时，客户端和后端可能对同一 thread 给出不同状态；
- subagent 的 Go/Goal 仍是 `Active` 时，child completion 可能早于 Goal continuation 发送给 parent；
- child final status、completion 投递、parent 消费 pending input 的顺序不清晰时，parent 可能在消费 child
  completion 前就继续 goal continuation，或在 child final 后永远不继续。

## 目标

- 用统一状态表达 turn complete 之后的阶段：`ThreadActive -> GoContextContinuation /
  ThreadIdle(WaitChild | WaitCommand) / ThreadCompletion`。`ThreadIdle` 只表示当前没有 turn 在跑、没有
  active goal continuation，但仍等待 command event 或 direct child completion 这两类外部唤醒。
- post-turn 检查顺序固定为：pending input -> active goal continuation -> incomplete direct child ->
  wait command -> complete。
- child 只对 direct parent 负责；递归 completion 由 child 在本地完成后向自己的 parent 逐层投递，不由
  grandparent 递归扫描。
- 普通 non-management subagent 本地满足完成条件后先投递 child completion message；parent 启动 turn
  消费该 pending input 时，才把该 direct child 标记为 parent-visible complete。
- `agent_mode = management` 不参与 parent completion delivery；本地完成条件满足时可以直接 `ThreadCompletion`。
- 无 pending input 且 Goal `Active` 时注入 Go context，不发送 child completion；incomplete direct child、
  running command 或 event subscription 不阻止 active goal continuation。
- Goal `Complete` / `Paused` / `BudgetLimited` / 不存在时才评估 wait child / wait command，并允许
  child completion 在本地完成条件满足后发送。
- 保留 child completion 只发送一次的 latch 语义。

## 状态模型

```rust
enum ThreadPostTurnState {
    ThreadActive,
    ThreadIdle(ThreadIdleReason),
    GoContextContinuation { goal_id: ThreadGoalId },
    ThreadCompletion,
}

enum ThreadIdleReason {
    WaitCommand,
    WaitChild,
}
```

- `ThreadActive`：当前 thread runtime 仍 active，包括 active turn、pending mailbox、queued input。
- `GoContextContinuation`：没有 active turn / pending input，且 Goal 仍为 `Active`，应注入
  `<goal_context>` 并启动 continuation；此时不发送 child completion，也不等待 direct child 或 command 先完成。
- `ThreadIdle(WaitChild)`：当前没有 turn 在跑且没有 active goal continuation，但 direct child 尚未 parent-visible complete，或 child
  completion 已投递但 parent turn 尚未消费。pending direct child completion 只有在 parent turn 消费对应 child
  completion pending input 后才解除；child 已经 final 但 parent 尚未消费 completion 时仍是 `WaitChild`。
- `ThreadIdle(WaitCommand)`：当前没有 turn 在跑且没有 active goal continuation，但仍有 running command 或
  active event command subscription 等待外部 command event 唤醒。
- `ThreadCompletion`：无 pending input、无 wait child、无 wait command，且 Goal 为 `Complete` / `Paused` /
  `BudgetLimited` / 不存在，可进入 child completion 发送判断。

## Go 状态分类

- `Active` -> `GoContextContinuation`
- `Complete` -> `ThreadCompletion`
- `Paused` -> `ThreadCompletion`
- `BudgetLimited` -> `ThreadCompletion`
- 无 Goal、Goals feature disabled、ephemeral thread 无 state db -> `ThreadCompletion`

## active 判定

post-turn gate 使用 session / agent control 的 runtime facts，但只看 direct child，不递归扫描 descendants。
递归等待由 direct child 自己的 completion gate 传递：如果 grandchild 仍 active，child 不会向 parent 投递
completion，parent 的 direct child pending completion 会继续保持 `WaitChild`。该 gate 使用：

- `pending_direct_child_completions`
- queued response items
- pending mailbox items
- `ThreadActive`：queued response items、pending mailbox items、active turn
- `GoContextContinuation`：Goal `Active`，且没有 active turn / pending input。
- `ThreadIdle(WaitChild)`：无 active goal continuation，且 `pending_direct_child_completions`、
  `AgentControl::direct_agent_children_are_active(self.conversation_id)`
- `ThreadIdle(WaitCommand)`：无 active goal continuation，且当前 thread 的 active event subscription count、running unified exec command

`direct_agent_children_are_active` 只枚举 direct thread-spawn children，并通过 `agent_thread_is_active` 对齐
thread status 使用的 canonical facts：active turn、active event subscriptions、非 final lifecycle status。
child completion 自身仍使用包含 self 的 `has_active_child_completion_work()`，保持“本地未完成不投递
parent completion”的 gate 语义。

## 后端状态与客户端展示

app-server canonical `ThreadStatus` 使用 `Active`、`Idle { reason: WaitChild | WaitCommand }`、`Complete`
表达加载后 runtime 状态。`NotLoaded` 和 `SystemError` 只作为加载/错误哨兵。root-worker Agent Tree 只消费当前
thread 后端返回的 canonical status；不得递归读取 children、conversation items、raw marker 或 agent text 来推导父
thread 的 running / waiting / complete。

## child completion 消费顺序

1. spawn/followup 给 direct child 时，如果 child 是普通 non-management agent，parent 记录
   `pending_direct_child_completions`。
2. child 本地 `ThreadCompletion` 后，向 direct parent 投递
   `InterAgentOperation::ChildCompletion`，并设置 `trigger_turn = true`。
3. parent 收到 inter-agent communication 时只入 mailbox 和触发 turn，不立即解除 pending direct child。
4. parent turn start drain queued response items 与 mailbox items；如果其中包含 child completion，则解除对应
   direct child pending completion。
5. parent turn finish 后再按 post-turn 顺序评估；此时若无 pending input 且 Goal `Active`，启动 goal
   continuation。若没有 active goal continuation，才继续评估 direct child / wait command / complete。

`list_agents` 或 `get_status` 看到 child lifecycle `Completed` 只说明 child 本地 lifecycle 已 final；它不等于
parent 已收到或消费 child completion。读取到 final lifecycle status 时可以触发一次幂等 recheck，但 recheck
仍必须走 `maybe_notify_parent_of_final_status` 和 typed `ChildCompletion` pending input，不得从 list/status 结果
直接把 child 标记为 parent-visible complete。

final status 的来源可以是完整 `send_event` 路径，也可以是 runtime 边界已经持久化的 `send_event_raw` typed
event。只要事件把 agent status 推进到 final，session 必须重新执行 current-source child completion 判定；这样
直接子 agent 即使没有后续显式 `wait_agent` / `list_agents` 调用，也能把 `ChildCompletion` 投递到 direct parent，
再由 parent 的 trigger-turn mailbox 启动消费和展示。重复 recheck 仍由 active 边沿 latch 去重。

## 测试

- active Goal 且 direct child 未 parent-visible complete：仍注入 Go context，不发送 child completion。
- 无 active Goal 且 direct child 未 parent-visible complete：进入 `ThreadIdle(WaitChild)`。
- active child final 但 parent 尚未消费 child completion：无 active Goal 时保持 `ThreadIdle(WaitChild)`。
- parent 消费 child completion pending input 后：可继续走 Go continuation 或 completion。
- grandchild active 时：child 不投递 completion，parent 通过 direct child pending completion 间接等待。
- active Goal 且 active event command subscription 或 running command 存在：仍注入 Go context，不发送 child completion。
- 无 active Goal 且 active event command subscription 或 running command 存在：进入 `ThreadIdle(WaitCommand)`。
- Goal `Active` 且无 pending input：注入 Go context，不发送 child completion。
- Goal `Complete` / `Paused` / `BudgetLimited` 且 thread idle：允许 child completion。
- `list_agents` / `get_status` 读到 final lifecycle status：只触发幂等 typed child completion delivery recheck，
  parent-visible complete 仍以 parent 消费 `ChildCompletion` pending input 为准。
- management agent 本地完成后不投递 parent completion，可直接进入 `ThreadCompletion`。
- repeated final/status recheck：child completion 仍只发送一次。
- raw final-status event：无需 parent 主动 `wait_agent` 或 `list_agents`，direct parent 会自动消费 typed child
  completion。

## 风险

如果 active event subscription 或 subagent lifecycle 未正确清零，无 active Goal 时 thread 会停留在
`ThreadIdle(WaitCommand)` / `ThreadIdle(WaitChild)`，child completion 也会被本地 completion gate 延后。
active Goal 的 continuation 不再等待这些 external work；显式等待仍应由模型通过 `wait_agent` /
`command_wait` 发起。

state db 打开或读取失败时，post-turn state 记录 warning 后 fail-open 到 `ThreadCompletion`。这个取舍保留
child completion 可达性，避免没有唤醒机制时永久阻塞；代价是极端情况下如果真实 Goal 仍为 `Active`，child
completion 可能早于 Go continuation。若后续需要 fail-closed，需要新增显式 deferred/retry 状态和可靠唤醒机制，
不要复用 `ThreadActive` 表达非 active 的数据库错误。

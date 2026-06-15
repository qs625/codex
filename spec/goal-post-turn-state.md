# Goal post-turn state

## 背景

Goal runtime 会在 active turn 清理后尝试自动启动下一轮 continuation，并注入
`<goal_context>`。child completion 则在 subagent 进入 final status 后通知 parent。两条链路都发生在
turn complete 之后，但原逻辑分别判断：Goal continuation 只看本 thread 是否没有 active turn /
queued input / trigger-turn mailbox，child completion 只看 child runtime 是否仍 active。这样会出现两类
问题：

- thread tree 仍有 active subagent、running command 或 event command 时，Goal context 可能提前注入；
- subagent 的 Go/Goal 仍是 `Active` 时，child completion 可能早于 Goal continuation 发送给 parent。

## 目标

- 用统一状态表达 turn complete 之后的阶段：`ThreadActive -> ThreadIdle ->
  GoContextContinuation / ThreadCompletion`。
- `ThreadActive` 判定复用现有 canonical recursive active helper，不复制 subagent、event command 或
  command running 条件。
- thread idle 且 Goal `Active` 时注入 Go context，不发送 child completion。
- thread idle 且 Goal `Complete` / `Paused` / `BudgetLimited` / 不存在时允许 child completion。
- 保留 child completion 只发送一次的 latch 语义。

## 状态模型

```rust
enum ThreadPostTurnState {
    ThreadActive,
    ThreadIdle,
    GoContextContinuation { goal_id: ThreadGoalId },
    ThreadCompletion,
}
```

- `ThreadActive`：当前 thread runtime 仍 active，包括 active turn、active subagent subtree、
  running command 所属 active turn、active event command subscription、pending mailbox、queued input、
  pending direct child completion。
- `ThreadIdle`：runtime active 条件已解除，可以继续检查 Goal 状态。
- `GoContextContinuation`：`ThreadIdle` 后发现 Goal 仍为 `Active`，应注入 `<goal_context>` 并启动
  continuation；此时不发送 child completion。
- `ThreadCompletion`：`ThreadIdle` 后发现 Goal 为 `Complete` / `Paused` / `BudgetLimited` / 不存在，
  可进入 child completion 发送判断。

## Go 状态分类

- `Active` -> `GoContextContinuation`
- `Complete` -> `ThreadCompletion`
- `Paused` -> `ThreadCompletion`
- `BudgetLimited` -> `ThreadCompletion`
- 无 Goal、Goals feature disabled、ephemeral thread 无 state db -> `ThreadCompletion`

## active 判定复用

`ThreadActive` 不复制具体条件。post-turn active gate 复用 session / agent control 的 canonical runtime
facts，但会排除当前 thread lifecycle 自身，避免新线程 `PendingInit` 阻塞首轮用户输入。该 gate 使用：

- `pending_direct_child_completions`
- queued response items
- pending mailbox items
- 当前 thread 的 active event subscription count
- `AgentControl::agent_descendants_are_active(self.conversation_id)`

`agent_descendants_are_active` 递归检查 live descendants，并通过 `agent_thread_is_active` 对齐 thread status
使用的 canonical facts：active turn、active event subscriptions、非 final lifecycle status。child completion
自身仍使用包含 self 的 `has_active_child_completion_work()`，保持原有 final/completion gate 语义。

## 测试

- turn complete 但 active subagent subtree 存在：不注入 Go context，不发送 child completion。
- active subagent final 后：parent 可进入 idle，并继续走 Go continuation 或 completion。
- turn complete 但 active event command subscription 存在：不注入 Go context，不发送 child completion。
- Goal `Active` 且 thread idle：注入 Go context，不发送 child completion。
- Goal `Complete` / `Paused` / `BudgetLimited` 且 thread idle：允许 child completion。
- repeated final/status recheck：child completion 仍只发送一次。

## 风险

如果 active event subscription 或 subagent lifecycle 未正确清零，Goal continuation 和 child completion 都会按
canonical active 状态被延后。这与 root-worker `ThreadStatus` / child completion gate 的语义一致；应优先修复
active facts 生命周期，而不是在 Goal 或 completion 中绕过 active 判断。

state db 打开或读取失败时，post-turn state 记录 warning 后 fail-open 到 `ThreadCompletion`。这个取舍保留
child completion 可达性，避免没有唤醒机制时永久阻塞；代价是极端情况下如果真实 Goal 仍为 `Active`，child
completion 可能早于 Go continuation。若后续需要 fail-closed，需要新增显式 deferred/retry 状态和可靠唤醒机制，
不要复用 `ThreadActive` 表达非 active 的数据库错误。

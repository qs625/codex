# Current Work

## Current Goal
- 修正 child completion 与 thread status 的关系。
- 目标语义：`WaitChild` / `IdleWaitChild` 只反映 direct child thread 是否本地 active；parent-side pending child completion bookkeeping 只负责 completion envelope 的投递与清理。

## Current Status
- 状态：in_progress
- 当前仓库：`/Users/bytedance/Projects/my-codex-dev-3`
- 当前分支：`fix/child-completion-thread-status`
- 当前阶段：实现与最小验证已完成，待提交。

## Recent Progress
- 已将 `agent-runtime::ThreadPostTurnInputs` 的语义从 `has_incomplete_direct_child` 改为 `has_active_direct_child`。
- 已把 `thread-service` 的 `has_incomplete_direct_child()` 判定改为只查询 `direct_agent_children_are_active()`，不再把 pending completion bookkeeping 计入 child activity。
- 已补 `thread-service/src/agent/control_tests.rs` 两条新语义测试：
  - active direct child 仍然驱动 `WaitChild`
  - 仅有 pending child completion bookkeeping 时不再驱动 `WaitChild`
- 已更新 `project-understanding.md`，把这条状态机语义记为长期项目事实。
- 独立 reviewer 已通过，无阻塞问题；唯一额外建议是补一条更贴近真实生命周期的 integration-style 用例，覆盖 parent 完成后再消费 child completion envelope 的路径。
- 已跑过最小验证和 child completion 相关现有回归；现有 `turn_start_consumes_child_completion_before_parent_visible_complete` 已覆盖一条接近 reviewer 关注点的生命周期路径，因此本轮未再新增 integration-style 用例。

## Files Already Read
- `codex-rs/agent-runtime/src/thread_post_turn.rs`
  - 原因：`WaitChild` 的最终 post-turn 判定入口
  - 结论：这里是 canonical scheduler state 选择点，字段名和值语义应与项目长期规则一致
  - 是否还需再看：验证失败或 reviewer 提到状态机细节时回看
- `codex-rs/agent-runtime/src/child_completion_state.rs`
  - 原因：确认 pending child completion bookkeeping 的真实职责
  - 结论：它应只做 delivery active / pending completion 计数，不应定义 child 是否 active
  - 是否还需再看：通常不需要，除非要改投递去重逻辑
- `codex-rs/thread-service/src/session/events_history.rs`
  - 原因：parent thread 侧的 `has_pending_turn_input`、`has_active_direct_child`、child completion bookkeeping API 都在这里
  - 结论：本任务的最小实现点就在这里；pending bookkeeping 与 child activity 已可分离
  - 是否还需再看：需要，验证或修 review 时大概率回看
- `codex-rs/thread-service/src/goal.rs`
  - 原因：goal continuation 后的 idle/completion 仍会走 child/command wait 判定
  - 结论：这里必须同步使用新的 `has_active_direct_child` 语义
  - 是否还需再看：若 goal 相关测试失败再看
- `codex-rs/thread-service/src/agent/control.rs`
  - 原因：确认 `direct_agent_children_are_active()` 的定义
  - 结论：它查询 open direct children 并按 child thread runtime status 判断 active，符合用户要求的“只反映 child 本地生命周期”
  - 是否还需再看：通常不需要
- `codex-rs/thread-service/src/agent/multi_agent.rs`
  - 原因：确认 `followup_task`、close agent 与 pending completion bookkeeping 的交互
  - 结论：followup 时仍会 re-arm pending completion；这可以保留，因为它只影响 envelope 投递，不再影响 `WaitChild`
  - 是否还需再看：如果 followup 场景测试失败再看
- `codex-rs/thread-service/src/thread/manager.rs`
  - 原因：确认 child completion pending/received 的 runtime bridge
  - 结论：manager 仍会在 child completion 收到时通知 parent，但不需要再让这层决定 child activity
  - 是否还需再看：通常不需要
- `codex-rs/thread-service/src/thread/codex.rs`
  - 原因：`ThreadPostTurnState` 到 `ThreadRuntimeStatus` 的映射点
  - 结论：`IdleWaitChild` 仍然由 `WaitChild` 映射，不需要额外改 UI/status 层
  - 是否还需再看：除非 runtime status 断言失败
- `codex-rs/thread-service/src/agent/control_tests.rs`
  - 原因：最直接覆盖 parent/child lifecycle 和 `WaitChild` 语义
  - 结论：这里最适合放新语义测试
  - 是否还需再看：需要，跑最小验证时会继续用
- `codex-rs/thread-service/src/session/tests/context_and_history.rs`
  - 原因：已有 child completion 投递、消费、清理测试
  - 结论：现有用例已证明 bookkeeping 仍用于 envelope 流转；其中 `turn_start_consumes_child_completion_before_parent_visible_complete` 已跑通，可作为 parent 完成后消费 child completion 的近似生命周期覆盖
  - 是否还需再看：如果要补更端到端的验证，需要回看

## Key Conclusions
- 用户已确认接受 eventual consistency：child 已 `complete` 后再被 followup 唤醒期间，parent 暂时看到 child 仍是 `complete` 可以接受。
- 本任务不应让 parent-side pending completion bookkeeping 继续定义 child 当前状态或 `WaitChild`。
- 真实 child activity 的唯一来源应是 direct child thread runtime status；pending completion 只表示“一个已完成 child 还有 envelope 尚未被 parent 消费”。

## Files Likely Safe To Skip
- `apps/root-worker-prototype/`
  - 本任务没有 UI 改动要求。
- `codex-rs/app-server-protocol/`
  - 当前没有新增 protocol/thread item 需求。
- `codex-rs/tool-service/`
  - 本任务不是 tool surface 变更。

## Current Blockers
- 无阻塞。

## Next Steps
- 提交当前分支改动。
- 向 PM 回报验证结果、review 结论和 commit hash。

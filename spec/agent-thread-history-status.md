# Agent child completion 单一生产和历史状态恢复

## Brief

用户在 root-worker prototype 中观察 parent/child agent 对话时，child completion 可能重复出现、出现在 conversation 底部，或在 child/subtree 仍 active 时过早显示 completed。已完成 worker thread 还可能因为 loaded status 与 turns 不一致显示为 Active，或者 thread/read/resume 后把 live completion 和历史 completion 显示成两份。

追加复核发现：这些现象主要在 compact 后暴露。compact 后 root-worker 会把 compact 前 conversation cells 归档到 `Previous conversation`，并把 `contextCompaction.replacementHistory` 作为当前模型上下文展开到主列表。这是 compact 的模型上下文设计，不是 rollout/thread history 丢失；用户误判来自归档入口不够明确，以及 replacement history、live item、read/resume snapshot 中的同一 child completion 没有在展示层统一去重。

成功标准：

- child thread/subtree 仍 active 时不发送 completed child completion。
- child thread/subtree 首次变为 inactive final 时只发送一个 completion status item。
- canonical visible item 使用 `collabAgentStatusUpdate`；legacy `collabAgentMessage(childCompletion)` 只作为历史兼容输入。
- parent conversation 中 completion 按 parent active turn 或通知位置正常插入，不固定到底部。
- live item、mailbox/history item、thread/read/resume snapshot 语义相同的 completion 只显示一次。
- compact 后旧 conversation 通过 `Previous conversation` archive row 可展开，compact replacement history 与 live/read completion 不双显。
- `thread/read` 对 loaded thread 的历史恢复与 `thread/resume` 保持一致，会用 listener 中真实 in-progress turn 修正状态。
- 已加载且已有 turn 数据的 thread 只有在存在 active turn work、active event monitor、in-flight subagent wait，或尚未加载 turns 的 active placeholder 时才展示为 Active。
- 保持 live thread/local state 模型；切换 thread 不新增无条件 `thread/read`。

非目标：

- 不修改 Markdown agent frontmatter 行为。
- 不调整模型选择或 TUI。
- 不重构 conversation 渲染结构。

## 技术设计

生产路径：

- 保留 core `forward_child_completion_to_parent` 为唯一生产者，输出带 `status` 的 `InterAgentCommunication(ChildCompletion)`。
- `maybe_notify_parent_of_final_status` 继续用 final status、非 management、无 active subtree/pending work 做 gate。
- completion 发送改为 active 边沿触发：新 turn 或仍 active 时记录 active，首次从 active 变为 inactive 时发送；重复 final 复检不会重复发送。
- 当前 thread 派发给 direct child 的 work 未收到 child completion 入账前，当前 thread 仍视为 active，避免 parent 在 direct child completion 尚未入账前提前向自己的 parent 发送 completion。
- active event subscription 是 child 是否 active 的后端事实，不能只靠 app-server watch observer 维护。`file-subscription` registry 在注册、完成、取消 subscription 时直接同步 `ThreadManager.active_event_subscriptions()`；app-server observer 只更新 UI/watch 状态。
- active event subscription 从非零变为零时，由 registry 通过 `ThreadManager::maybe_notify_parent_of_final_status` 补发一次复检，避免 final 时被 event command gate 挡住后漏发。

根因修正：

- 旧路径中 event command 已经登记在 `file-subscription` registry，但 core `ActiveEventSubscriptionTracker` 只通过可选 app-server observer 更新。observer 不存在、未覆盖或顺序竞争时，`agent_thread_is_active()` 看到 active count 为 0，`maybe_notify_parent_of_final_status` 会把仍等待 event command 的 child 当作 inactive，从而过早发送 child completion。
- registry 现在是 active event subscription 的 single writer：新增/移除 subscription 后计算 registry 内真实 active count，同步 core tracker，再通知 app-server watch manager。这样 parent completion gate 读取的是后端真实状态，不依赖客户端展示层去重或掩盖。

历史和展示：

- parent mailbox 保留，用于模型输入和历史兼容；live visible item 通过 app-server 归一为 `collabAgentStatusUpdate`。
- root-worker 不再用 stable semantic key、raw marker 或 legacy JSON envelope 去重 typed `ThreadItem`；live synthetic completion 与 thread/read/resume 还原出的 item 只有在 `ThreadItem.id` 相同时才视为同一 item。
- root-worker 对 compact `replacementHistory` 中的 legacy serialized `childCompletion` 不再作为展示层去重依据；重复展示应回到 projector 或后端事件源修复，客户端只保证 typed entry 保真。
- compact 前 cells 继续进入 archive row，不改写为完整主列表；compact row 文案明确旧消息在 `Previous conversation`，replacement/current context 在下方继续。
- 同一 turn 内重复 terminal `collabAgentStatusUpdate` 如有不同 id，会作为不同 typed item 保留并生成 entry。
- legacy `collabAgentMessage(childCompletion)` 仍可加入 active parent turn，但不作为新的 canonical visible shape。

测试覆盖：

- event subscription gate 单测需要先终止 `spawn_agent` 自动提交的初始 child turn，再人工模拟 `TurnComplete`。否则 `has_active_child_completion_work()` 会因为 active subtree 仍存在而继续阻止 completion，这是正确 gate 行为，不是 event subscription 清零后的补发缺陷。
- `multi_agent_v2_completion_waits_for_active_event_subscription` 覆盖：active subscription count 为 1 时不发 completion；count 清零并复检后补发 completion。
- `multi_agent_v2_restored_event_subscription_blocks_completion_until_cleared` 覆盖：恢复出的 active subscription 会阻止 final completion，清零后只补发一次。

thread/read 状态恢复：

- loaded thread 的 `thread/read` 会读取 `ThreadState.active_in_progress_turn_snapshot()`。
- `includeTurns=true` 时将 persisted rollout turns 与真实 in-progress snapshot 合并。
- `includeTurns=false` 和 persisted metadata 快路径也使用真实 in-progress snapshot 修正 status，但不构造 turns。

前端最小改动：

- 保留 `decideThreadSelectionAction` 的懒加载语义，不在切换时强制 read。
- 后端 `ThreadStatus` 是 root-worker Agent Tree 的 canonical 状态来源。`Active.activeFlags` 只表达 `running`、`waitingOnApproval`、`waitingOnUserInput`；`Idle.reason` 表达 `waitChild` 或 `waitCommand`；`Complete` 表示无 pending input、无 wait child、无 wait command 且无立即启动的 goal continuation。root-worker 只做当前 thread status 到现有视觉 class/label 的映射，不递归 children 推导父状态。
- app-server 在 turn start/complete、event subscription count、subagent wait begin/end、approval/user input guard 变化时统一重算 status 并发送 `thread/status/changed`。event command / running command 等待输出 `Idle { reason: waitCommand }`，direct child 未 parent-visible complete 或 subagent wait 输出 `Idle { reason: waitChild }`。
- parent 等待 subagent 的 turn 已完成后，`CollabWaitingEnd` 若携带 completed agent status，会进入短暂 grace window 并继续保持 `waitChild`；matching child completion 的 live `ItemCompleted` 到达后，按其 `trigger_turn` 精确切换为 `running` 或 `complete`。若 child completion item 先到，则先标记 `running`，后续 wait-end 清除 `waitChild`；若 child completion item 缺失，grace 过期后清除等待，避免事件顺序差异造成 idle gap、stale running 或 stale waiting。
- root-worker 不再从 turn/items/raw markers 推导 `doing`、`waiting-subagent`、`waiting-eventtool` 或 idle；旧 item 只用于 conversation/monitor 内容展示，不作为 Agent Tree 主状态事实。

## 风险

- 如果后端或 projector 继续用不同 id 发送内容相同的 completion，root-worker 会如实显示重复 entry；这符合 typed item 保真约束，重复源头需要在上游修复。
- 如果后端持续错误报告 event subscription active，child completion 会继续被 gate 挡住，直到 active count 归零。
- 未加载 thread 仍依赖服务端 status，避免 list/tree 中尚未加载的 active child 被误降级。

# Agent thread 历史和状态恢复

## Brief

用户在 root-worker prototype 中选中已完成的 worker agent thread 时，conversation 只剩 `Initial context injected` 和 child status item，早前消息消失；同时 header 仍显示 `Worker Agent • Active`，但可见 child status 已是 `shutdown` / completed。

成功标准：

- 选中已完成或 shutdown 的 agent thread 时，已有 conversation 历史不被 live status item、subscribe snapshot 或 read snapshot 覆盖。
- `thread/read` 对 loaded thread 的历史恢复与 `thread/resume` 保持一致，会合并 listener 中尚未进入 rollout 的 active turn snapshot。
- 已加载且已有 turn 数据的 thread 只有在存在 active turn work、active event monitor 或 in-flight subagent wait 时才展示为 Active。
- 保持 live thread/local state 模型；切换 thread 不新增无条件 `thread/read`。

非目标：

- 不修改 Markdown agent frontmatter 行为。
- 不调整模型选择或 TUI。
- 不重构 conversation 渲染结构。

## 技术设计

服务端最小改动：

- 在 `thread/read` 的 loaded thread 路径读取 `ThreadState.active_turn_snapshot()`。
- `includeTurns` 时复用 `populate_thread_turns_from_history`，像 `thread/resume` 一样把 persisted rollout turns 与 active snapshot 合并。
- active status 的强制提升仍只服务于真实 in-progress turn，避免把普通 loaded thread 当作 active turn。

前端最小改动：

- 保留 `decideThreadSelectionAction` 的懒加载语义，不在切换时强制 read。
- 新增 thread-level presence/status class 派生：当 thread 已有 turn 数据但没有 active work、active monitor、in-flight subagent wait 时，`status.type === "active"` 不再直接显示为 Active。
- tree status 仍允许无 turns 的 active placeholder 显示 active，避免未加载 live child 在树上丢失运行态。

## 风险

- 如果后端持续错误报告 event subscription active，但前端 turns 中没有 monitor，已加载 conversation 会显示 Idle；这是本次用户可见回归的目标行为。
- 未加载 thread 仍依赖服务端 status，避免 list/tree 中尚未加载的 active child 被误降级。

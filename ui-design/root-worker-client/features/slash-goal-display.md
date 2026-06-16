# Feature: Slash Goal Display

## 目标

让用户在 root-worker prototype 中通过 slash command 管理 thread goal，并能在 thread 页面持续看到当前 goal 状态和内容。

## 设计结论

- 复用现有 composer slash menu，不引入新的 command palette。
- 在 thread header 下方新增 `GoalStrip`，只在 selected thread 存在 goal state 时显示。
- 在 Thread Analysis 中新增 `GoalDetailPanel`，承载完整 goal、预算、最近事件和取消入口。
- `/goal cancel` 是一等内置 command；无 active goal 时显示 disabled reason 或轻量反馈。
- `/init` 不作为 root-worker builtin command；它来自 system skill discovery。
- 数据来源必须是 typed goal state/API 或 typed `ThreadItem`，不得解析 agent 文本、raw marker 或 legacy envelope。

## Prototype

- [slash-goal-display-prototype.svg](/Users/bytedance/Projects/my-codex/.worktrees/slash-goal-display/ui-design/root-worker-client/assets/slash-goal-display-prototype.svg)

## Slash Command 增量

Commands 分组建议顺序：

1. `/goal cancel`：Cancel the current thread goal。
2. `/clear`：Archive this root session and start a fresh root。
3. `/init`：来自 system skill，不由 builtin command registry 特判。

搜索：

- 输入 `/goal`：显示 `/goal cancel`。
- 输入 `/cancel`：显示 `/goal cancel`。
- 输入 `/init`：显示 system skill `/init`。

执行：

- `/goal cancel` 推荐直接执行；若需要确认，第一版不要弹 modal，改用可撤销的后端语义或明确不可撤销文案。当前需求未要求二次确认。

## Goal 状态

Header strip：

- 无 goal：不显示。
- active：`Goal active` + 摘要 + budget + cancel。
- paused：`Goal paused` + 摘要 + cancel。
- complete：`Goal complete`，可短暂展示或只留 detail 历史。
- budget limited：`Budget limited` + 摘要 + budget emphasis + cancel。
- cancelling：禁用 cancel，显示 `Cancelling`。
- cancel failed：显示 inline error，保留重试。

取消状态模型：

- `cancelling` 是 transient action state，可以由本地 pending action 表达，但必须以 `threadId + actionId` 管理，并由 RPC 返回或 typed lifecycle event 清除。
- `cancel failed` 的错误原因来自 action error result 或 typed goal lifecycle item，不能从 agent message text 解析。
- 后端如果提供 `goal/cancelRequested`、`goal/cancelled`、`goal/cancelFailed` 等 typed `ThreadItem`，UI 同时用它更新 Goal Detail 的最近事件。

Right Panel detail：

- 无 goal：`No active goal.`
- active/paused/budgetLimited：完整内容、预算、最近 lifecycle event。
- complete/cancelled：作为最近 event 保留，不抢占 active 信息。

## 空态与错误

- 无 selected thread：composer 和 Right Panel 继续使用当前空态；Goal UI 不出现。
- app-server 不可用：顶部已有全局错误；Goal Detail 可显示 `Goal state unavailable`，不做额外 modal。
- cancel race：如果 goal 已被其他 continuation 完成或取消，返回 `No active goal to cancel`，strip 按最新 typed state 更新。
- cancel 请求失败：保留原 goal，不清空 UI，错误靠近触发点展示。

## 开发 handoff

实现入口建议：

- `apps/root-worker-prototype/src/lib/slashMenu.ts`：扩展 command id、command metadata、disabled reason。
- `apps/root-worker-prototype/src/components/Panels.tsx`：command row disabled/aria-disabled、`/goal cancel` 执行反馈。
- `apps/root-worker-prototype/src/App.tsx`：扩展 `runComposerSlashCommand`，接入 goal cancel API；不要把 slash command 当普通 message 发送给模型。
- `apps/root-worker-prototype/src/types.ts`：如果后端已有 typed goal payload，则在 `Thread` 上增加 `goal` 或等价 typed state；如果是 lifecycle item，则仍通过 `ThreadItem` 投影进入 conversation/detail。
- `apps/root-worker-prototype/src/components/RightPanel.tsx`：Thread Analysis 增加 `GoalDetailPanel`。
- 可新增 `components/GoalStrip.tsx` 或在 `Panels.tsx` 附近拆出，避免继续膨胀中心组件。

工程约束：

- 不从 `agentMessage.text`、raw marker、legacy envelope、JSON 文本中反解 goal。
- Goal 状态不应混入 `ThreadStatus.activeFlags`；`ThreadStatus` 继续只表达 thread active state。
- live 更新只按 typed id/state 合并，不因 goal 内容相同而去重 `ThreadItem`。

## 验收清单

- Slash menu 展示 `/goal cancel`，`/init` 通过 system skill 出现，搜索和键盘选择可用。
- Active goal 时 header strip 与 Thread Analysis detail 同步更新。
- 无 goal 时 `/goal cancel` 不会产生误导性成功反馈。
- cancel 中、成功、失败都有明确视觉状态。
- 窄宽度下 goal 摘要不挤压 composer 或右侧 rail。

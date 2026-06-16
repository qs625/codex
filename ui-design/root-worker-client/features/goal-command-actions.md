# Feature: Goal Command Actions

## 目标

在 root-worker prototype 已有 GoalStrip、GoalDetailPanel 和 `/goal cancel` 基线上，补齐 `/goal <objective>`、`/goal pause`、`/goal resume`、`/goal cancel|clear` 的轻量交互 handoff。用户应能在 composer 内完成 goal 的创建/更新、暂停、恢复和取消，同时通过 GoalStrip 与 Thread Analysis 判断当前状态。

## 范围

涉及：

- Composer slash menu 的 goal command family 文案、搜索、补全与执行反馈。
- Composer draft parser 对 `/goal <objective>`、`/goal pause`、`/goal resume`、`/goal cancel|clear` 和 `/cancel-goal` 兼容别名的区分。
- GoalStrip 与 GoalDetailPanel 的 action 状态、空态和错误反馈。
- typed goal API / notification 的 UI 消费边界。

不涉及：

- 新增 `/goal-cancel`、`/goal-pause` 等顶层主命令。
- 从 `agentMessage.text`、raw marker、legacy JSON envelope 或 assistant message JSON 解析 goal 状态。
- 重做 Right Panel、conversation cell 或 Agent Tree 视觉结构。

## Baseline 与 Prototype

Baseline：

- [baseline-goal-command-actions-2026-06-16.png](/Users/bytedance/Projects/my-codex/.worktrees/goal-command-actions/ui-design/root-worker-client/assets/baseline-goal-command-actions-2026-06-16.png)

Prototype：

- [goal-command-actions-prototype.svg](/Users/bytedance/Projects/my-codex/.worktrees/goal-command-actions/ui-design/root-worker-client/assets/goal-command-actions-prototype.svg)

截图说明：baseline 使用 `$root-worker-playwright-debug` 完整 Electron smoke 获取，`window.codexDesktop` 可用；当前本地 app-server bootstrap 失败，因此截图覆盖真实 Electron shell、三栏布局、composer 和右侧 rail，不覆盖真实 goal data state。

## Slash Menu Handoff

Commands 分组建议顺序：

| Label | Description | 可用性 | 选择行为 |
| --- | --- | --- | --- |
| `/goal <objective>` | Set or update the current thread goal | 有 selected thread 时可用 | 补全 `/goal `，保持 focus，等待用户输入 objective |
| `/goal pause` | Pause the active thread goal | goal 为 active/budgetLimited 且后端允许 pause | 补全 `/goal pause`，用户再次 Enter 后执行 |
| `/goal resume` | Resume the paused thread goal | goal 为 paused | 补全 `/goal resume`，用户再次 Enter 后执行 |
| `/goal cancel` | Cancel the current thread goal | goal 为 active/paused/budgetLimited | 补全 `/goal cancel`，用户再次 Enter 后执行 |
| `/clear` | Archive this root session and start a fresh root | 有 root session 时可用 | 沿用现有 clear 行为 |

搜索与别名：

- `/goal`：显示完整 goal command family，`/goal <objective>` 排第一。
- `/goal c`、`/cancel-goal`：命中 `/goal cancel`。
- `/clear goal` 可作为搜索 alias 命中 `/goal cancel`，但不改变 `/clear` 自身“归档当前 root session 并新建 root”的语义；parser 不能把 `/clear goal` 当作 goal action 执行。
- `/goal clear`：作为 `/goal cancel` 的 parser alias；菜单可在 meta 显示 `Alias: /goal clear`，不新增独立主行。
- `/cancel-goal`：仅兼容旧输入，不作为主展示 label。

二级 query 规则：

- 现有 `getActiveComposerSlashQuery` 遇到空格会关闭菜单；本 feature 需要允许首行 `/goal` 后的二级 query 保持菜单打开。
- 建议解析为 `{ root: "goal", subquery: string }`：`/goal ` 打开 goal scoped menu，`/goal p` 过滤 pause，`/goal r` 过滤 resume。
- 排序规则：空 subquery 时 `/goal <objective>` 第一；subquery 命中保留 subcommand 前缀时，优先选中对应 subcommand；其他非空内容才回落为 objective。
- `/goal <objective>` 与 `/goal pause/resume/cancel/clear` 的执行判定必须优先匹配保留 subcommand；其他非空内容才视为 objective。

键盘和鼠标行为：

- `ArrowUp/ArrowDown`：移动 active candidate。
- `Tab`：补全 active candidate，不执行 action。
- `Enter` 且 menu 打开：补全 active candidate，不执行 action。
- `Enter` 且 draft 已是完整 `/goal pause`、`/goal resume`、`/goal cancel` 或 `/goal clear`：执行对应 typed action。
- 鼠标点击候选：补全 active candidate，不执行 action。

## Composer 执行反馈

- set/update pending：`Setting goal...`
- set/update success：`Goal updated.`
- set/update empty objective：`Enter a goal objective.`
- set/update failure：`Could not set goal: <reason>`
- pause pending：`Pausing goal...`
- pause unavailable：`No active goal to pause.`
- pause success：`Goal paused.`
- pause failure：`Could not pause goal: <reason>`
- resume pending：`Resuming goal...`
- resume unavailable：`No paused goal to resume.`
- resume success：`Goal resumed.`
- resume failure：`Could not resume goal: <reason>`
- cancel pending：`Cancelling goal...`
- cancel unavailable：`No active goal to cancel.`
- cancel success：`Goal cancelled.`
- cancel failure：`Could not cancel goal: <reason>`

反馈来源必须是 action result、typed goal state 或 typed lifecycle item；不能解析 assistant 文本。Composer status 使用 `role=status` 或 `aria-live=polite`，覆盖 pending、success、failure 和 unavailable 文案。

## GoalStrip

状态：

- 无 goal 且无 action error：不显示。
- active：显示 `Goal active`、objective 摘要、token/time usage、Pause 与 Cancel action。
- paused：显示 `Goal paused`、objective 摘要；Resume 是 primary action，Cancel 是 secondary/overflow action。
- budgetLimited：显示 `Budget limited`、预算强调、Pause/Cancel action 按后端能力展示。
- complete：显示 `Goal complete` 或仅在 Thread Analysis 作为最近事件保留；不显示 destructive action。
- pending：badge 可显示 `Pausing`、`Resuming`、`Cancelling`，禁用重复 action。
- error：保留原 goal 内容，在触发点附近显示结构化错误。

布局：

- 宽屏可显示两个文本按钮或 icon+text action。
- 中等宽度只保留一个 primary action，次要 action 进入 overflow 或 Thread Analysis。
- 窄宽度保留 badge、单行摘要和固定 32px icon action，预算隐藏到 detail。

## Thread Analysis / GoalDetailPanel

Goal Detail 放在 Thread Analysis 顶部，优先展示：

- 状态 badge。
- 完整 objective。
- token/time/turn budget。
- 最近 typed goal lifecycle event。
- 操作区：Pause、Resume、Cancel、Copy goal、Edit goal。

`Edit goal` 可把 `/goal <current objective>` 写入 composer 并聚焦；如果当前 composer 有未发送内容，需要确认或保留现有 draft，避免覆盖用户输入。

## 数据与工程边界

- root-worker 展示只能消费 typed `ThreadGoal`、goal action API result、`thread/goal/updated` / `thread/goal/cleared` 或后续 typed lifecycle `ThreadItem`。
- `ThreadStatus.activeFlags` 不承载 goal active/paused；它只表达 thread runtime active state。
- `/goal <objective>`、pause、resume、cancel 都不得作为普通 user message 发送给模型。
- `/cancel-goal` 兼容 parser 应进入同一个 `goalCancel` action，不产生单独 command id。
- 建议用统一 `goalActionStateByThreadId` 管理 pending/error，而不是为 pause/resume/cancel 分别维护数组和 map。

## 开发 Handoff

建议入口：

- `apps/root-worker-prototype/src/lib/slashMenu.ts`：扩展 command id、goal scoped query、alias 和 disabled reason。
- `apps/root-worker-prototype/src/lib/composerDraft.ts`：新增 goal command parser，返回结构化 action `{ type, objective? }`。
- `apps/root-worker-prototype/src/App.tsx`：扩展 `runComposerSlashCommand` 和 `sendMessage` 前置拦截，接入 typed set/pause/resume/cancel API。
- `apps/root-worker-prototype/src/components/Panels.tsx`：slash menu 二级 query、GoalStrip action 状态和 aria label。
- `apps/root-worker-prototype/src/components/RightPanel.tsx`：GoalDetailPanel action 区、Edit/Copy 入口和 lifecycle event 展示。
- `apps/root-worker-prototype/src/types.ts` / `electron.d.ts`：按 app-server v2 最终协议补 typed goal action payload。

## 验收

- `/` 与 `/goal` 均可发现 goal command family，菜单主展示保持 `/goal <subcommand>` 风格。
- `/cancel-goal` 输入仍可取消 goal，但菜单不新增 `/cancel-goal` 主行。
- `/goal <objective>` 非空时创建/更新 goal，空 objective 不发送普通消息。
- pause/resume/cancel 在无对应 goal 状态时给明确 disabled reason 或 composer feedback。
- GoalStrip 与 GoalDetailPanel 对 active、paused、budgetLimited、complete、pending、error 状态有清晰反馈。
- 所有 goal 展示与反馈来自 typed state/API/notification，不解析 raw/assistant 文本。

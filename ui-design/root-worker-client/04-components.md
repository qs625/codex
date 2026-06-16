# Components

## CommandExecutionCell

职责：展示一个 command session 的 canonical 记录。

状态：

- `running` / `inProgress`
- `waitingOutput`
- `waitingExit`
- `completedSuccess`
- `completedFailed`
- `declined` / `failed`

内容：

- Header：command、cwd、status、duration。
- Details Execution：command、cwd、status、duration、exit code。
- Details Session：initial wait、notify on、yield time、tty、max output tokens、sandbox permissions、approval prefix/justification。
- Details Output：aggregated output 摘要和截断提示。

数据要求：

- 不从 `agentMessage.text`、raw marker 或 JSON envelope 解析。
- 若要展示 initial wait / notify on，typed `ThreadItem.commandExecution` 或 app-server projector 必须提供 session 参数。

## CommandNotificationEvent

职责：展示 command session 的 output/exit notification，和 command cell 的 live tail 明确区分。

- output notification：标题 `Command output notification`，摘要使用最新 chunk 的首行或末行。
- exit notification：标题 `Command exit notification`，摘要显示 `Exit N` 或 `Completed`。
- 必须携带 `targetCommandItemId` 或等价 typed reference。

## SlashCommandMenu

职责：composer 输入 `/` 时提供内置命令和 Skills 的发现、过滤、补全和选择。

子组件：

- `SlashMenuOverlay`：锚定弹层、定位、最大高度和滚动容器。
- `SlashMenuGroup`：`Commands` / `Skills` 分组标题和分组状态。
- `SlashMenuItem`：可选择候选行，支持 active、hover、disabled、loading/error 附近状态。
- `SkillChip`：composer 内结构化 skill token，复用现有 chip 视觉和 payload 行为。

## ComposerSlashMenuCommand

位置：复用 `Panels.tsx` 中现有 `SlashMenuGroup` / `SlashMenuOption`。

新增字段建议：

- `commandId`: `clear` | `goalInit` | `goalCancel`
- `token`: `clear` | `goal cancel`
- `label`: `/clear` | `/goal cancel`
- `description`: 一句短说明。
- `aliases`: 支持 `goal`, `init`, `cancel`, `stop goal` 等搜索。
- `disabledReason`: 可选，展示在 meta；有值时不可执行。

状态：

- default：白底透明，hover/selected 使用现有 amber selected 背景。
- selected：保留 `.composer-slash-option.selected`。
- disabled：降低文字对比但保持可读，鼠标 cursor default，`aria-disabled=true`。
- empty：复用 `No commands or skills match...`，当 query 为 `/goal` 时显示 `/goal cancel`。

行为：

- `/init` 不进入 command metadata；它来自 system skill。
- `/goal cancel`：选择后直接执行取消；若无 active goal，不执行并显示 disabled reason。

## GoalStrip

位置：Main Thread Panel 的 header 下方、Conversation Virtual List 上方。

结构：

- 左侧 status badge。
- 中间 goal summary，两行截断。
- 右侧 budget summary 和 icon actions。

状态：

- active：badge 使用 amber/green 混合中的 amber accent，文本 `Goal active`。
- paused：neutral badge，文本 `Goal paused`。
- complete：success badge，文本 `Goal complete`。
- budgetLimited：warning badge，文本 `Budget limited`。
- cancelling：badge 文本 `Cancelling`，cancel button disabled。
- cancelFailed：右侧 inline error，保留 cancel button。

取消状态来源：

- `cancelling` 可以来自本地 action pending state，但必须 keyed by thread id 和 action id，并在 RPC resolve/reject 或 typed lifecycle event 到达后清除。
- `cancelFailed` 来自 action error result 或 typed `goal/cancelFailed` lifecycle item；错误原因必须是结构化字段。
- 如果后端提供 `cancelRequested` / `cancelFailed` typed `ThreadItem`，Goal Strip 与 Goal Detail 都消费同一 typed item，不从 agent message 解析。

交互：

- 点击 strip 非按钮区域打开 Thread Analysis 并定位到 Goal Detail。
- Cancel icon button 有 tooltip 和 aria-label。
- 长内容截断，完整内容进入 detail panel；不要把 `title` 当作 disabled/error 原因的唯一承载。

## GoalDetailPanel

位置：Right Panel 的 Thread Analysis 视图顶部，优先级高于 monitors/context usage。

内容：

- 标题：`Goal`
- 状态 badge。
- 完整 goal 内容。
- Budget rows：`Turns`、`Tokens`、`Time`，仅展示后端提供的字段。
- Recent event：最近 typed goal lifecycle item。
- 操作：Cancel goal、Copy goal。

取消 action 字段：

- `canCancel`: boolean，由 canonical goal status 和本地 pending state 派生。
- `disabledReason`: `No active goal`、`Cancel already requested`、`Goal complete` 等结构化原因。
- `lastActionError`: 结构化错误文本，只来自 API/action result 或 typed lifecycle item。

空态：

- 无 selected thread：`Select a thread to inspect goal state.`
- 无 goal：`No active goal.`
- 加载中：只显示标题和 skeleton 行，不阻塞其他 Thread Analysis 内容。

## Feedback

Composer status：

- `Cancelling goal...`
- `Goal cancelled.`
- `No active goal to cancel.`
- `Could not cancel goal: <reason>`

所有反馈文案应由 command/action 结果驱动；如果后端提供 typed goal event，则 conversation/detail 也展示同一事件。

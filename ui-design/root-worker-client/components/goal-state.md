# Component: Goal State

## Goal 数据模型需求

前端需要的最小 typed state：

- `status`: `active` | `paused` | `complete` | `budgetLimited` | `cancelled`
- `text`: goal 内容。
- `createdAt` / `updatedAt`: 可选，用于 detail。
- `budget`: 可选，包含 turns/tokens/time 的 used/remaining/limit。
- `lastEvent`: 可选，typed lifecycle event 摘要。
- `canCancel`: boolean 或由 status 派生。
- `canPause`: boolean 或由 status/backend capability 派生。
- `canResume`: boolean 或由 status/backend capability 派生。
- `disabledReason`: 可选，结构化原因，例如 `No active goal`、`Cancel current goal first`、`Cancel already requested`。

取消 action 不强制进入 canonical goal `status`，但必须有 typed 来源：

- Pending：本地 `goalActionState` keyed by `threadId + actionId`，或 typed `goal/cancelRequested` lifecycle item。
- Success：goal state 更新为 `cancelled` 或 typed `goal/cancelled` lifecycle item。
- Failure：action error result 或 typed `goal/cancelFailed` lifecycle item，包含结构化 `reason`。

UI 可以用 pending/error action state 派生 `cancelling` 和 `cancelFailed` 视觉状态；这些状态不得从 assistant text、raw marker 或 legacy envelope 推断。

创建/更新、暂停、恢复 action 同样不强制进入 canonical goal `status`，但必须有 typed 来源：

- Set/update pending：本地 `goalActionState` keyed by `threadId + actionId`，成功后以 typed goal state 更新为准。
- Pause pending：本地 pending 或 typed `goal/pauseRequested` lifecycle item；成功后 `status=paused`。
- Resume pending：本地 pending 或 typed `goal/resumeRequested` lifecycle item；成功后 `status=active`。
- Failure：action error result 或 typed lifecycle item，包含结构化 `reason`。

推荐前端维护统一 `goalActionStateByThreadId`，字段包括 `action: set | pause | resume | cancel`、`pending`、`error`、`actionId`，避免为每个动作扩散独立数组和错误 map。

字段命名以 app-server v2 最终协议为准，但必须是 typed payload，不依赖文本解析。

## 视觉规格

- Strip 高度：内容短时 44-52px；长内容最多两行，不推动 conversation 大幅下移。
- Badge：小号圆角 pill，不能只靠颜色区分，必须有文本。
- Cancel：使用现有 icon button 风格；可用 `StopIcon` 或新增明确的 close/ban icon。
- Detail panel：使用 Right Panel 当前 section/card 样式，不做大面积彩色背景。

## 文案

- `Goal active`
- `Goal paused`
- `Goal complete`
- `Budget limited`
- `Cancelling`
- `Pausing`
- `Resuming`
- `Cancel goal`
- `Pause goal`
- `Resume goal`
- `Set goal`
- `No active goal.`
- `Could not cancel goal: <reason>`

## 可访问性

- Goal Strip 使用 `region` 或明确 aria-label：`Thread goal`。
- 状态 badge 文本直接可读。
- Cancel button disabled 时保留原因：`title` 或邻近 inline text。
- 所有 action feedback 使用 `role=status` 或 `aria-live=polite`，包括 pending、success、failure 和 unavailable；错误反馈也走同一 live region 规则。

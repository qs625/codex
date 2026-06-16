# Component: Goal State

## Goal 数据模型需求

前端需要的最小 typed state：

- `status`: `active` | `paused` | `complete` | `budgetLimited` | `cancelled`
- `text`: goal 内容。
- `createdAt` / `updatedAt`: 可选，用于 detail。
- `budget`: 可选，包含 turns/tokens/time 的 used/remaining/limit。
- `lastEvent`: 可选，typed lifecycle event 摘要。
- `canCancel`: boolean 或由 status 派生。
- `disabledReason`: 可选，结构化原因，例如 `No active goal`、`Cancel current goal first`、`Cancel already requested`。

取消 action 不强制进入 canonical goal `status`，但必须有 typed 来源：

- Pending：本地 `goalActionState` keyed by `threadId + actionId`，或 typed `goal/cancelRequested` lifecycle item。
- Success：goal state 更新为 `cancelled` 或 typed `goal/cancelled` lifecycle item。
- Failure：action error result 或 typed `goal/cancelFailed` lifecycle item，包含结构化 `reason`。

UI 可以用 pending/error action state 派生 `cancelling` 和 `cancelFailed` 视觉状态；这些状态不得从 assistant text、raw marker 或 legacy envelope 推断。

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
- `Cancel goal`
- `No active goal.`
- `Could not cancel goal: <reason>`

## 可访问性

- Goal Strip 使用 `region` 或明确 aria-label：`Thread goal`。
- 状态 badge 文本直接可读。
- Cancel button disabled 时保留原因：`title` 或邻近 inline text。
- 错误反馈使用 `role=status` 或 `aria-live=polite`。

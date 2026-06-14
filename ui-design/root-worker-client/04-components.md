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

类型：
- output notification：标题 `Command output notification`，摘要使用最新 chunk 的首行或末行，详情可展开看 chunk 摘要。
- exit notification：标题 `Command exit notification`，摘要显示 `Exit N` 或 `Completed`，详情显示 duration、exit code、关联 command。

关联：
- 必须携带 `targetCommandItemId` 或等价 typed reference。
- UI 通过该 id 提供 “Back to command” 定位。

## LiveCommandRow

职责：RightPanel 中的 command session 快速索引。

显示：
- command label。
- cwd detail。
- status pill。
- latest notification：优先显示最新 typed notification 摘要；没有 notification 时显示 latest output tail。

交互：
- 行主点击：定位 command cell。
- 可选二级 affordance：定位 latest notification event。
- hover/focus 显示可点击状态。
- 禁用态用于目标不在本地 conversation cache 的情况。

可访问性：
- 可点击行使用 `button` 或带键盘 handler 的语义元素。
- `aria-label` 包含 command、cwd、status 和目标，如 `Jump to command: cargo test, Exit 101`。
- 高亮目标必须有非颜色线索，例如左侧强调线和 `aria-live` 简短提示。

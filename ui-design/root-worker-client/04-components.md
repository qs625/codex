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

## SlashCommandMenu

职责：composer 输入 `/` 时提供内置命令和 Skills 的发现、过滤、补全和选择。

子组件：
- `SlashMenuOverlay`：锚定弹层、定位、最大高度和滚动容器。
- `SlashMenuGroup`：`Commands` / `Skills` 分组标题和分组状态。
- `SlashMenuItem`：可选择候选行，支持 active、hover、disabled、loading/error 附近状态。
- `SkillChip`：composer 内结构化 skill token，复用现有选择、展示、删除和 payload 行为。

状态：
- `closed`
- `openIdle`：刚输入 `/`，展示默认候选。
- `filtering`：有 query，候选即时过滤。
- `skillsLoading`：Commands 可用，Skills 分组加载中。
- `skillsError`：Commands 可用，Skills 分组显示失败。
- `empty`：Commands 与 Skills 均无匹配。

行为：
- `Up` / `Down`：在可见可选 item 中移动 active descendant。
- `Enter`：选择 active item；内置命令执行 command id，Skill 走现有选择/chip/payload 行为。
- `Tab`：补全或选择 active item，不发送普通消息；Skill 走现有 chip/payload 行为。
- `Escape`：关闭菜单并保留 draft。
- 鼠标点击：选择 item；hover 更新 active。

数据要求：
- 内置命令：本次至少需要 `commandId`、`token`、`label`、`description`。本次验收只覆盖无参数命令。
- Skill：沿用当前 `name`、`path` 数据；如果后续透传 app-server metadata，可增补 `description`、`source`、`available`、`errorReason`。
- 过滤基于结构化字段，不解析 conversation 文本。

可访问性：
- composer 使用 combobox 模式表达 expanded、controls 和 active descendant。
- menu list 使用 listbox/options 或等价 ARIA 语义。
- active item 必须有非颜色高亮：背景、左侧强调线或清晰边框。
- loading/error 状态使用 `aria-live="polite"`，避免每次输入重复播报完整列表。

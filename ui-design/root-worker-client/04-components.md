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

## GoalLifecycleEventCell

职责：展示模型 goal 工具或 goal runtime 产生的 user-visible typed lifecycle item，作为 conversation 的可回溯事实记录。

数据来源：

- typed `ThreadItem.goalLifecycle` / `ThreadItem.goalEvent` 或等价 v2 typed payload。
- 不从 `agentMessage.text`、raw marker、legacy envelope、tool output JSON 或 `<goal_context>` 反解。

推荐字段：

- `id`: typed item id，同时作为 `ConversationEntry.id`。
- `action`: `create` | `update` | `read` | `pause` | `resume` | `cancel` | `complete` | `budgetLimited` | `failed`。
- `status`: `active` | `paused` | `complete` | `budgetLimited` | `cancelled` | `failed` | `checked`。
- `objectivePreview`: 120-160 字符摘要。
- `objective`: 可选完整内容；默认只给 RightPanel 使用。
- `reason`: 可选结构化失败原因。
- `goalId`: 可选，用于 RightPanel/状态关联，不用于 conversation 去重。
- `createdAtMs` / `completedAtMs`: 用于时间。

ConversationEntry 映射：

- `kind`: 复用 `event`。
- `role`: `system`。
- `text`: 标题 + 摘要，例如 `Goal created: Add typed goal ThreadItem display...`。
- 建议新增 `eventCategory: "goal"` 和 `eventStatus`，让 `EventRow` 切换图标、badge、两行布局；如果短期不扩类型，至少不能把 goal lifecycle 映射为普通 agent message。

视觉：

- 左侧 goal/target/check 图标，使用现有 icon button 体系或新增简洁 line icon。
- 主体 pill 保持 event row 低噪音尺寸；标题和 badge 第一行，objective preview 第二行。
- created/updated 使用 amber accent；complete 使用 green badge；paused/checked 使用 neutral；failed 使用 red badge 和结构化原因文本。
- 长 objective 两行截断，完整内容进入 `GoalDetailPanel`。
- 布局收缩优先级：badge 不压缩；title 使用 `min-width: 0` 单行截断；time 可下移到 meta 行，窄宽度可隐藏到 `aria-label` / tooltip；objective preview 使用两行 clamp。
- 圆角和阴影跟随现有 event row，不做独立大 card。

状态与合并：

- 同一 `ThreadItem.id` 的 started/completed 更新可合并为同一 cell。
- 不同 id 必须保留为不同 entry，即使 objective 和 status 相同。
- 连续 goal events 不合并成一条 summary；视觉可以相邻，但 entry 边界必须保留。
- compact replacement history 中按 archive 内 event row 展示，不丢弃 terminal item。

虚拟列表与定位：

- 如果 `EventRow` 从单行扩展为 goal 两行布局，必须更新 `conversationVirtualization` 的 event/goal event 高度估算，或确保测量后稳定修正。
- 搜索跳转、RightPanel recent event 跳转和 archived compact 内部展示都要覆盖两行 goal event，避免定位后高亮错位。
- RightPanel recent event 使用 typed item id 定位，控件语义为 button/link，支持 Enter/Space 与 focus ring。

## WorkflowProgressCell

职责：展示 Dynamic Workflow 运行图和进度，作为 conversation 中的 typed workflow timeline item。

数据来源：

- typed `ThreadItem.workflowRunProgress` / `ThreadItem::WorkflowRunProgress` 或等价 v2 typed payload。
- primary display path：`EventMsg::WorkflowRunProgressCompleted -> ThreadItem::WorkflowRunProgress`。
- 不从 `agentMessage.text`、raw marker、assistant JSON、workflow tool output、`workflow/run/updated` debug text 或 legacy envelope 反解。

推荐字段：

- `id`: typed item id，同时作为 `ConversationEntry.id`。
- `runId`: workflow run id；显示可截短，例如 `run abc123`，不用于 conversation 去重。
- `workflowId`: stable workflow id，例如 `feature-dev`。
- `workflowName`: 用户可读名称，例如 `Feature Development`。
- `status`: `queued` | `running` | `waiting` | `completed` | `failed` | `aborted`。
- `currentStageId`: 可选，当前 stage。
- `message`: 可选 typed summary，例如 `Implement stage running`。
- `stages`: 有序数组，包含 `id`、`label`、`status`、`agentLabel`、`startedAtMs`、`completedAtMs`、`errorMessage`。
- `edges`: 可选 static graph edge；最小版本可忽略复杂布局，只按 stage 顺序展示。
- `updatedAtMs`: 用于时间。

视觉：

- Header 左侧使用流程/节点类 line icon；标题显示 workflow name，副信息显示短 run id 和更新时间。
- 状态 badge：queued/waiting 使用 neutral，running 使用 accent，completed 使用 success，failed 使用 danger，aborted 使用 muted warning。
- Stage rail：桌面横向，每个 stage 为固定最小宽度 segment，包含短 label 和状态 chip；segment 之间用细线连接。
- 窄屏纵向 list：每个 stage 一行，左侧状态点，右侧 label + 状态，避免横向滚动。
- 当前 stage 可用更深边框或轻背景强调；不要使用动画作为唯一进行中信号。
- 错误原因显示在失败 stage 下方一行，最多两行截断；完整内容进入 tooltip/aria-label 或 future detail。

状态文案：

- queued：`Workflow queued`
- running：`Workflow running`
- waiting：`Workflow waiting`
- completed：`Workflow completed`
- failed：`Workflow failed`
- aborted：`Workflow aborted`
- typed fallback：`Workflow updated` + `Progress details unavailable`

空/缺字段处理：

- 无 `stages`：显示 header + summary + `No graph details in this update.`，但仍保留 typed item。
- 无 `workflowName`：使用 `workflowId`；都缺失时显示 `Workflow run`。
- 无 `currentStageId`：running 状态显示第一条 `running` stage；仍找不到时只显示 run status。
- 无 `message`：根据 status 和 current stage 合成短摘要，但不能解析 raw output。

行为与合并：

- 同一 `ThreadItem.id` 的 started/completed 更新可合并为同一 cell。
- 不同 `ThreadItem.id` 必须保留为不同 entry，即使 `runId` 相同。
- 如果实现希望展示“最新 run progress”聚合，必须放在 RightPanel summary；conversation timeline 不做 destructive merge。
- compact replacement history 中按 typed id 和 server 顺序保留 workflow progress item。

可访问性：

- cell 的整体 aria-label 包含 workflow name、status、current stage 和 stage 总数。
- 每个 stage 的状态有文字和 aria-label，例如 `Research, Done`、`Review/Fix, Running`。
- Stage rail 不依赖颜色区分完成/失败；badge 文本必须可见。
- 如支持 stage 详情展开，stage control 使用 button 语义；最小版本只读，无需把每个 stage 加入 Tab 顺序。

## WorkflowThreadBadge

职责：在 Agent Tree 或 conversation header 中轻量标识当前 thread / agent 属于哪个 workflow run，不改变树的主结构。

数据来源：

- 优先消费 thread metadata 中的 workflow binding，例如 `workflowId`、`workflowName`、`runId`、`stageId`、`stageLabel`、`role`。
- 可从 typed thread/session metadata、`ThreadStatus` 附带 metadata 或 thread read payload 获取；具体字段名由后端协议定稿。
- 不从 thread path、agent message、subagent name、workflow progress card 文案或 raw envelope 反解所属关系。

推荐字段：

- `workflowId`: stable workflow id。
- `workflowName`: 用户可读名称。
- `runId`: workflow run id，UI 截短。
- `stageId` / `stageLabel`: 当前 thread 对应 stage，例如 `Review/Fix`。
- `bindingRole`: `root` | `stageAgent` | `reviewer` | `tester` | `worker` | `unknown`。
- `bindingStatus`: `active` | `waiting` | `complete` | `failed` | `unknown`。

Agent Tree 形态：

- 在 agent label 的第二行或右侧显示小 badge，例如 `Workflow · feature-dev · Review/Fix`。
- badge 文本优先使用 `stageLabel`，tooltip/aria-label 显示完整 `workflowName` 和截短 run id。
- 不新增 workflow 分组，不把树改成 graph，不把同一 run 的 agents 重新排序。
- 多个 workflow binding 时只显示当前 active / latest binding；完整列表留给未来 Thread Analysis。

Conversation Header 形态：

- 在 path / role / run config 附近显示一枚低权重 chip，例如 `Feature Development · Review/Fix · run wf_42a9`。
- chip 点击可定位到最近 `WorkflowProgressCell`，前提是有 typed item id；没有定位目标时只读，不表现为按钮。
- 可点击和只读状态必须视觉区分：有定位目标时才显示 button hover/focus 样式；没有定位目标时保持静态 chip，不使用 pointer cursor。
- chip 只表达所属关系，不展示进度百分比，也不替代 progress cell。

缺 metadata 处理：

- 无 workflow binding metadata 时不显示 badge，避免误判。
- 如果当前 conversation 已有 workflow progress card，但 thread metadata 无 binding，可在设计/开发验收中记录断点：`Progress is visible, but thread-to-workflow binding metadata is unavailable.`；UI 不从 progress card 反推当前 thread 所属关系。
- Debug 模式或 Thread Analysis 可显示低权重提示 `No workflow binding metadata for this thread.`；默认主界面不制造噪音。

## CommandWaitReplacementEntry

职责：在 compact / replacement history 中展示一次 `command_wait` 的语义结果，避免把普通 `function_call_output` JSON 暴露给用户。

内容：

- 标题：`Waited for command`。
- 关联 command：短 command label 或可定位的 typed command reference。
- 状态：`Completed`、`Output received`、`Still running`、`Timed out`、`Command unavailable`。
- notification：`output`、`exit`、`completed` 或 `none`；只表达本次 wait window 的命中结果。
- exit code：仅 command exit/completed 且字段存在时显示。
- wall time：优先显示本次 wait 实际耗时；若只具备 command 总时长，字段名使用 `Command duration`。
- wait timeout：显示本次 current window，例如 `Wait window 1s`，不得展示 hard cap。

行为：

- 不默认展示 stdout/stderr 全量内容。
- 不展示 raw `command_id`、JSON 字段名、普通 tool output 或 `Function call <call_id>` start 行。
- 可定位时通过 `ThreadItem.id` / `targetCommandItemId` 定位，禁止文本匹配。
- typed item 缺失时显示低权重 fallback event，不回退到 JSON。

## CommandWriteStdinReplacementEntry

职责：在 replacement history 中展示 `command_write_stdin` 的语义动作。

内容：

- 标题：`Sent input to command`。
- 摘要：`Wrote stdin to running command`。
- 关联 command。
- 输入摘要：行数或字符数；默认不展示完整 stdin。
- 结果状态：`Sent`、`Command unavailable`、`Rejected`。

安全：

- stdin 内容可能包含 secret 或用户输入，默认只展示摘要。
- 如未来提供 details/debug 展开，必须复用既有 redaction 策略。
- 如果 typed payload 暂时没有异常结果状态，UI 默认只表达 `Sent`；异常状态必须来自后端 typed 字段，不从 raw output 推断。

## CollabWaitReplacementEntry

职责：在 replacement history 中展示 `wait_agent` 的语义等待结果。主路径应消费 `CollabWaitingBegin` / `CollabWaitingEnd` typed lifecycle item，不展示 raw tool JSON。

内容：

- 标题：`Waiting for subagent`、`Subagent update received`、`Subagent completed` 或 `No subagent update during this wait window`。
- target agent label/path。
- update 类型：message、child completion、status changed、timeout。
- wait timeout：本次 current window。

fallback：

- 如果只有 `wait_agent` raw output 而没有 typed lifecycle item，展示 `Waited for subagent` + `No typed subagent wait event was recorded for this history entry.`。
- fallback 仍不展示 JSON、tool name、call id 或 arguments。

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

- `commandId`: `clear` | `goalSet` | `goalPause` | `goalResume` | `goalCancel`
- `token`: `clear` | `goal` | `goal pause` | `goal resume` | `goal cancel`
- `label`: `/clear` | `/goal <objective>` | `/goal pause` | `/goal resume` | `/goal cancel`
- `description`: 一句短说明。
- `aliases`: 支持 `goal`, `init`, `cancel`, `stop goal` 等搜索。
- `disabledReason`: 可选，展示在 meta；有值时不可执行。

状态：

- default：白底透明，hover/selected 使用现有 amber selected 背景。
- selected：保留 `.composer-slash-option.selected`。
- disabled：降低文字对比但保持可读，鼠标 cursor default，`aria-disabled=true`。
- empty：复用 `No commands or skills match...`；当 query 为 `/goal` 时应显示 goal command family，而不是进入 empty。

行为：

- `/init` 不进入 command metadata；它来自 system skill。
- `/goal <objective>`：选择后补全 `/goal ` 并保持 composer focus，不立即执行；Enter 仅在 objective 非空时调用 typed goal set/update action。
- `/goal pause` / `/goal resume`：无参数 action command。菜单 Enter 或鼠标点击只补全完整 token 并保持 composer focus；用户再按 Enter 执行。
- `/goal cancel`：菜单 Enter 或鼠标点击只补全 `/goal cancel` 并保持 composer focus；用户再按 Enter 执行。若无 active/paused/budgetLimited goal，候选不可执行补全并显示 disabled reason。
- `Tab`：补全 active candidate，不执行任何 goal action。
- `/goal clear`：作为 `/goal cancel` 的 parser alias；可显示在 `/goal cancel` meta 中，不建议作为独立主行。
- `/cancel-goal`：兼容别名；可搜索命中 `/goal cancel`，不作为菜单 label。

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
- Paused goal 可显示 Resume action；active goal 可显示 Pause action。空间不足时 GoalStrip 只保留一个 primary action 和 overflow，完整 action 进入 Goal Detail。
- Paused goal 的 GoalStrip primary action 是 Resume，Cancel 作为 secondary/overflow；active/budgetLimited goal 的 primary action 是 Pause，Cancel 作为 secondary/overflow。
- 长内容截断，完整内容进入 detail panel；不要把 `title` 当作 disabled/error 原因的唯一承载。

## GoalDetailPanel

位置：Right Panel 的 Thread Analysis 视图顶部，优先级高于 monitors/context usage。

内容：

- 标题：`Goal`
- 状态 badge。
- 完整 goal 内容。
- Budget rows：`Turns`、`Tokens`、`Time`，仅展示后端提供的字段。
- Recent event：最近 typed goal lifecycle item。
- 操作：Pause goal、Resume goal、Cancel goal、Copy goal、Edit goal。

Goal actions：

- Set/update goal 不放在 detail panel 的主操作里，避免和 composer 输入目标内容的路径冲突；detail panel 可提供 `Edit goal`，点击后把 `/goal <current objective>` 填入 composer 并聚焦。
- Pause：active/budgetLimited 可用，pending 文案 `Pausing...`。
- Resume：paused 可用，pending 文案 `Resuming...`。
- Cancel/Clear：active/paused/budgetLimited 可用，pending 文案 `Cancelling...`。
- Complete 状态只保留 Copy，不提供 pause/resume/cancel，除非后端显式支持 reopen。
- Action feedback 容器使用 `role=status` 或 `aria-live=polite`，确保键盘和屏幕阅读器用户能感知 pending、success 和 failure。

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

- `Setting goal...`
- `Goal updated.`
- `Could not set goal: <reason>`
- `Pausing goal...`
- `Goal paused.`
- `Could not pause goal: <reason>`
- `Resuming goal...`
- `Goal resumed.`
- `Could not resume goal: <reason>`
- `Cancelling goal...`
- `Goal cancelled.`
- `No active goal to cancel.`
- `Could not cancel goal: <reason>`

所有反馈文案应由 command/action 结果驱动；如果后端提供 typed goal event，则 conversation/detail 也展示同一事件。

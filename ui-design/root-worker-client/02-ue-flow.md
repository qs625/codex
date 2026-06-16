# UE Flow

## 既有 Command Session 主路径

1. agent 发起 `exec_command`。
2. conversation 插入 command cell，显示命令摘要和 running/in progress 状态。
3. command cell 可展开查看 session details：命令、工作目录、状态、时长、退出码、output 摘要和 session 参数。
4. command 运行中或等待 notification 时，Right Panel 的 Live Commands 显示该条 session。
5. `output` 或 `exit` notification 到达时，conversation 追加独立 typed notification event，用文案说明这是来自同一 command session 的通知，而不是 command cell 本身的展开内容。
6. 点击 Right Panel 条目，conversation 滚动到对应 command cell 或最近一条关联 notification，并短暂高亮。
7. 成功完成后 Live Commands 自动移除；失败完成后作为近期失败保留。

## 既有 Composer Slash 菜单主路径

1. 用户在 composer 当前支持的 slash 触发位置输入 `/`。
2. composer 上方打开 slash menu，焦点仍在 composer。
3. 菜单按 `Commands`、`Skills` 分组展示。
4. 用户继续输入查询，菜单即时过滤名称、别名和简短说明。
5. `Down` / `Up` 在可见候选中循环移动，跳过分组标题和禁用行。
6. `Enter` 选择 active 候选；鼠标点击选择对应候选。
7. 选择内置命令时，使用候选的稳定 `commandId` 执行语义动作。
8. 选择 Skill 时，沿用当前 skill slash 行为：添加对应 Skill chip/attachment，payload 继续走现有结构化 skill 输入链路。
9. `Escape` 关闭菜单，保留 composer 中的 `/query` 文本和 selection。

## 主路径：查看 active goal

1. 用户选择一个 thread。
2. 客户端从 typed thread state / v2 payload 获取 `goal`。
3. 如果 goal 为 active，thread header 下方显示 Goal Strip。
4. Goal Strip 展示状态 badge、目标摘要、预算/continuation 摘要和 `Cancel` icon button。
5. 用户需要更多信息时，切换右侧 Thread Analysis；Goal Detail 展示完整 goal 内容、状态、预算和最近 goal lifecycle event。

## 主路径：通过 slash command 创建或更新 goal

1. 用户在 composer 首行输入 `/goal ` 后继续输入目标内容，例如 `/goal finish the typed command actions handoff`。
2. 当 draft 匹配 `/goal <objective>` 且 `<objective>` 非空时，send 不走普通 user message，而是调用 typed goal create/update action。
3. 发送中 composer 显示 `Setting goal...`，send button 禁用；draft 保留到 action 成功。
4. 成功后清空 draft，Goal Strip 显示 `Goal active` 和新 objective，Thread Analysis Goal Detail 同步完整内容。
5. 如果当前已有 active/paused goal，后端语义决定是 update 同一 goal 还是替换 goal；UI 文案使用 `Goal updated.`，避免暗示创建了新的 thread item。
6. 失败时 composer status 显示 `Could not set goal: <reason>`，保留 draft，错误原因来自 action result 或 typed lifecycle item。

## 主路径：通过 slash menu 发现 goal actions

1. 用户输入 `/`，Commands 分组展示 `/goal <objective>`、`/goal pause`、`/goal resume`、`/goal cancel`、`/clear`。
2. 用户输入 `/goal` 时，slash menu 保持打开并过滤到 goal command family；`/goal <objective>` 始终排第一，提示用户继续输入目标内容。
3. 用户输入 `/goal p` 时，候选优先为 `/goal pause`；输入 `/goal r` 时候选优先为 `/goal resume`。空 subquery 时 `/goal <objective>` 第一；subquery 命中保留 subcommand 前缀时，优先选中对应 subcommand；其他非空内容才回落为 objective。
4. `/goal clear` 作为 `/goal cancel` 的 alias 展示在 meta 或 secondary token，不作为单独主行，除非实现需要显示两个等价动作。
5. `/cancel-goal` 可搜索命中 `/goal cancel`，但候选 label 仍为 `/goal cancel`。
6. `/init` 由 system skill discovery 出现在 Skills 分组，不作为 root-worker builtin command。

## 主路径：通过 slash command 取消 goal

1. 用户输入 `/goal cancel` 或从 slash menu 选择该命令。
2. 如果当前 thread 有 active/paused/budgetLimited goal，命令可执行；否则 command row disabled，并在 meta 显示 `No active goal`。
3. 执行后 composer status 显示 `Cancelling goal...`，send button 与 command row 暂时禁用；这个 pending 状态来自本地 action result lifecycle， keyed by `threadId + goalActionId`，直到取消 RPC 返回或 typed goal lifecycle event 到达。
4. 成功后 Goal Strip 消失或转为短暂 `Cancelled` 状态，Thread Analysis Goal Detail 记录最近取消事件。
5. 失败后 Goal Strip 保持原状态，composer status 显示来自 action result 或 typed lifecycle item 的失败原因，并允许重试；不得从 assistant text 中提取错误。

## 主路径：暂停与恢复 goal

1. 用户从 slash menu 选择 `/goal pause` 时，composer 补全为 `/goal pause` 并保持 focus；用户再次按 Enter 后执行。直接输入 `/goal pause` 后 Enter 也执行。
2. 如果当前 goal 为 active 或 budgetLimited 且后端允许暂停，调用 typed pause action；GoalStrip button/command row 进入 pending 状态。
3. 成功后 GoalStrip badge 更新为 `Goal paused`，Thread Analysis 展示最近 event：`Paused just now`。
4. 如果当前没有 active goal，composer status 显示 `No active goal to pause.`，GoalStrip 不出现。
5. 用户从 paused 状态执行 `/goal resume`，成功后 badge 更新为 `Goal active`；无 paused goal 时反馈 `No paused goal to resume.`。
6. pause/resume 失败时不改变当前 goal state，靠近触发点显示 `Could not pause goal: <reason>` 或 `Could not resume goal: <reason>`。

## 主路径：从 Goal Strip 取消

1. 用户点击 Goal Strip 右侧 `Cancel goal` icon button。
2. Button 进入 busy 状态，aria-label 变为 `Cancelling goal`。
3. 成功后 Goal Strip 显示 2-3 秒 `Cancelled` inline feedback 后收起；如果后端保留 terminal goal state，则显示 `Cancelled` badge 且不显示 cancel button。
4. 失败时在 strip 右侧显示 inline error，按钮恢复可用。

## 状态覆盖

- 无 thread：不显示 Goal Strip；composer placeholder 保持 `Select an agent...`。
- thread notLoaded：Goal Strip skeleton 不显示，Right Panel 只显示 `Goal unavailable until thread loads`。
- 无 goal：header 不显示 strip；Thread Analysis Goal Detail 可显示 `No active goal`。
- active：显示 accent badge、目标摘要、预算/continuation。
- paused：显示 neutral badge；GoalStrip 至少显示 Resume 作为 primary action，Cancel 作为 secondary/overflow，GoalDetailPanel 展示 Resume 与 Cancel，并由 backend capability 控制 disabled reason。
- complete：显示 success badge；header 可短暂显示，之后进入 detail 历史。
- budgetLimited：显示 warning badge，突出 remaining/used budget；Cancel 可用。
- pausing/resuming：由本地 keyed action pending 或 typed lifecycle item 驱动，禁用同类重复 action，保留原 goal 内容。
- cancelling：由本地 action pending 或 typed `cancelRequested` lifecycle item 驱动，禁用重复操作，保留原 goal 内容。
- cancel failed：由 action error result 或 typed `cancelFailed` lifecycle item 驱动，保留原 goal 内容，显示短错误。

## 键盘与可访问性

- Slash menu 保持现有 ArrowUp/ArrowDown、Enter、Tab、Escape 行为。
- Goal action feedback 在 composer status 中使用 `role=status` 或 `aria-live=polite`，覆盖 `Setting goal...`、`Goal paused.`、失败原因等 action 结果。
- Disabled command row 仍可被屏幕阅读器理解：使用 `aria-disabled=true`，并在 meta 中给出原因；是否跳过键盘选中由实现决定，但不可执行。
- Goal Strip 的 cancel button 使用图标按钮加 tooltip/aria-label：`Cancel goal`。
- Goal 内容摘要使用文本，不只靠颜色表达状态。

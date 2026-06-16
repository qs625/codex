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

## 主路径：通过 slash command 初始化 goal

1. 用户在 composer 首行输入 `/` 或 `/goal`。
2. Slash menu 展示 Commands 分组：`/goal cancel`、`/clear`。
3. `/init` 由 system skill discovery 出现在 Skills 分组，不作为 root-worker builtin command。
4. `/goal cancel` 直接调用 typed goal clear action；不发送普通 user message。
5. 如果当前 thread 没有 goal，`/goal cancel` 反馈 `No active goal to cancel.`。
6. 提交有效内容后，composer 显示 sending 状态；成功后清空 draft，Goal Strip 更新为 active；失败时 composer status 显示错误并保留 draft。

## 主路径：通过 slash command 取消 goal

1. 用户输入 `/goal cancel` 或从 slash menu 选择该命令。
2. 如果当前 thread 有 active/paused/budgetLimited goal，命令可执行；否则 command row disabled，并在 meta 显示 `No active goal`。
3. 执行后 composer status 显示 `Cancelling goal...`，send button 与 command row 暂时禁用；这个 pending 状态来自本地 action result lifecycle， keyed by `threadId + goalActionId`，直到取消 RPC 返回或 typed goal lifecycle event 到达。
4. 成功后 Goal Strip 消失或转为短暂 `Cancelled` 状态，Thread Analysis Goal Detail 记录最近取消事件。
5. 失败后 Goal Strip 保持原状态，composer status 显示来自 action result 或 typed lifecycle item 的失败原因，并允许重试；不得从 assistant text 中提取错误。

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
- paused：显示 neutral badge，保留 Resume/Cancel 由后端能力决定；本 feature 只要求 Cancel。
- complete：显示 success badge；header 可短暂显示，之后进入 detail 历史。
- budgetLimited：显示 warning badge，突出 remaining/used budget；Cancel 可用。
- cancelling：由本地 action pending 或 typed `cancelRequested` lifecycle item 驱动，禁用重复操作，保留原 goal 内容。
- cancel failed：由 action error result 或 typed `cancelFailed` lifecycle item 驱动，保留原 goal 内容，显示短错误。

## 键盘与可访问性

- Slash menu 保持现有 ArrowUp/ArrowDown、Enter、Tab、Escape 行为。
- Disabled command row 仍可被屏幕阅读器理解：使用 `aria-disabled=true`，并在 meta 中给出原因；是否跳过键盘选中由实现决定，但不可执行。
- Goal Strip 的 cancel button 使用图标按钮加 tooltip/aria-label：`Cancel goal`。
- Goal 内容摘要使用文本，不只靠颜色表达状态。

# EventMsg 展示源迁移 Handoff

## 目标

EventMsg 将成为 app-server / root-worker UI 的 display source，但 root-worker 客户端的用户可见展示仍以 typed `ThreadItem` 投影为入口，并继续经过 `ThreadItem -> ConversationEntry -> ConversationCell` 渲染链路。

本次 handoff 的目标是保持现有 conversation、live update、thread/read、replacement history、RightPanel 定位和 Agent Tree 状态的用户体验不变，只替换后端 display source。它不是一次视觉改版，不新增页面、导航或组件形态。

## 基线与原型说明

本次不新增 baseline screenshot 或原型资产。

原因：这是后端 display source 迁移的文字级 UI/UE handoff，要求保持既有 UI 表现不变，不改变页面结构、视觉密度、组件布局或交互入口。现有 baseline 仍沿用：

- [baseline-command-session-2026-06-14.png](../assets/baseline-command-session-2026-06-14.png)
- [baseline-goal-threaditem-display-2026-06-16.png](../assets/baseline-goal-threaditem-display-2026-06-16.png)

如实现中改变 command/event/goal/workflow cell 的视觉形态、布局高度、状态图标或 RightPanel 行为，需要重新使用 `$root-worker-playwright-debug` 获取完整 Electron baseline 和状态截图。

## 用户可见不变量

- Conversation 是事实记录：所有可见历史项必须来自 typed `ThreadItem` 或等价 typed lifecycle payload，不从 assistant text、raw marker、legacy JSON envelope、tool output JSON、EventMsg debug text 反解。
- `ThreadItem.id` 是 conversation item 合并、去重、定位和 live 更新的唯一稳定键。不同 id 即使内容、状态、文本或时间相同，也必须保留为不同 `ConversationEntry`。
- 每个可见 typed item 至少生成一个 `ConversationEntry`。`ConversationCell` 只做视觉分组，不能跨 user/tool/event/schedule/child completion/workflow/goal 等语义边界丢 entry。
- live 模式中，已进入本地 live cache 的 thread 只能通过持续接收的 live typed item 更新；不能因为 EventMsg source 切换而触发 `thread/read` 后 destructive merge。
- `turn/started` / `turn/completed` 只更新 turn lifecycle metadata；不能把通知中的 `turn.items` 当 snapshot 覆盖 conversation items。
- user message 继续右对齐；连续普通 agent message 可以保持同一个 agent bubble 内的 segment 合并；tool/event/goal/workflow/child completion 不并入 agent bubble。
- Search、过滤、定位和高亮只能基于已投影的 `ConversationEntry` / `ConversationCell`；不得用 EventMsg raw payload、message text 或 JSON 内容参与去重或定位。
- RightPanel 是实时索引和跳转面板，不替代 conversation 详情；点击定位不展开 details、不抢 composer focus、不覆盖 draft。
- Agent Tree 主状态继续消费 canonical `ThreadStatus` / `thread/status/changed`，不从 EventMsg、conversation item 或 raw marker 推断 running / waiting / idle。

## 状态、合并、排序与 id 约束

### command_wait / command_write_stdin / command notification

- `exec_command` 的 command cell 继续代表完整 command session lifecycle；output/exit notification 必须继续作为独立 typed event 展示，不能折叠进 command cell 后丢失唤醒事实。
- `command_wait` 调用开始和结束必须是同一个 typed wait item id 的 started/completed lifecycle；UI 显示本次 current wait window，不显示 hard cap。
- `command_wait` completed 后展示本次命中结果：output、exit、completed、timeout 或 still running。不能从 stdout/stderr 内容推断 notification 类型。
- `command_write_stdin` 必须显示为独立 typed action item，默认展示输入摘要、行数/字符数和结果状态；不默认展示完整 stdin，尤其不能泄露 secret。
- command notification 通过 typed command item id 关联原 `CommandExecution`。RightPanel row 点击、notification 的 back-link、replacement history 定位都使用 id，不用 command 文本匹配。
- live tail 可以更新 command cell 的最新输出摘要，但 `ExecCommandOutputDelta` 不能被反解成新的 conversation event。

### collab / child completion

- subagent notification、inter-agent communication、child completion 和 `wait_agent` waiting begin/end 继续是 typed collab lifecycle item。
- `wait_agent` 每次等待显示本次 current wait window；timeout 后返回 running 并推进下一次窗口，UI 不展示 hard cap。
- child completion 只表达 direct child 对 parent 的完成投递；不要把 grandchild 或递归扫描结果合并成 parent-visible completion。
- 同一 child completion 的 started/completed 更新使用同一个 typed item id；不同 subagent 的同类更新不能按 target name 或文本合并。
- parent pending input/mailbox 中已有 typed update 时，等待展示应表现为“已有更新被消费”，不能显示成新的 raw JSON 工具输出。

### goal

- GoalStrip 继续表达当前 thread goal state；conversation goal item 继续表达可回溯 lifecycle 事件；RightPanel Goal Detail 继续表达完整内容、预算和最近事件。
- `create_goal`、`update_goal(status=complete)`、pause/resume/cancel/budget limited 等可见事件必须经 typed goal lifecycle item 投影。`get_goal` 默认不制造噪音，只有后端显式投影 user-visible read/check item 时才显示。
- goal lifecycle item 不能按 objective、status 或 goalId 合并；合并只看 `ThreadItem.id`。
- Goal terminal state 不能只表现为 GoalStrip 消失；conversation 中需要保留 `Goal complete` / cancelled / budget limited 等 terminal event。

### workflow

- workflow list/describe/start/status/resume/abort 的用户可见进度继续是 typed workflow progress item，不展示 TS runner raw output、assistant JSON 或 workflow tool output JSON。
- workflow start/resume/abort 这类 user action 必须保留独立 conversation entry；terminal update 可以更新同一 run 的 progress item，也可以追加新的 typed terminal item，但规则必须由 stable id 明确。
- workflow graph / stage / agent binding 的展示顺序以 typed progress event 的 created/completed time 和 server 顺序为准；客户端不能按 stage name、agent id 或文本重新排序。
- workflow run id 用于关联 RightPanel 或详情，不用于替代 `ThreadItem.id` 做 conversation 去重。

### event-command

- slash command、本地 runtime action、schedule/event-command 等展示必须继续走 typed event-command item；不能作为普通 user message 发送后再从 assistant 回复或 EventMsg 文本里补展示。
- `/clear`、`/goal ...` 等本地命令执行结果要么更新本地 composer/action state，要么产生 typed lifecycle item；失败原因需要结构化字段。
- event-command 的可见 started/completed/failed 状态不能按 command token 合并。例如连续两次 `/goal pause` 即使文案相同，也应保留不同 id 的事件。

### schedule

- schedule subscribe / unsubscribe / fired / failed / terminal update 必须继续走 typed schedule lifecycle item 或等价 typed `ThreadItem` 投影，不渲染 raw EventMsg、tool output JSON、cron 文本或 assistant message。
- 同一 subscription 的多次 fired event 必须保留为不同 `ThreadItem.id` 的 conversation entries；不能按 schedule label、cron 表达式、weekday/time 文案、subscription id 或 fire time 文本合并。
- subscribe / unsubscribe / failed / terminal 事件需要保留独立 conversation entry；recurring schedule 的 fired event 不应覆盖 subscribe entry，也不应被 unsubscribe entry 删除。
- RightPanel 若展示 active/recent schedules，只能通过 typed subscription id 关联状态，通过 `ThreadItem.id` 定位 conversation item；不可用 label 或 prompt 文本搜索定位。
- replacement history 中的 schedule fired / unsubscribe / failed 仍按 typed id 和 server 顺序保留，不能因为同属一个 recurring schedule 而压缩成一条 summary。

## 排序规则

- 默认按 app-server 已 canonicalize 的 typed item 顺序渲染，不在客户端按 EventMsg kind 做二次重排。
- started/completed 是同一 item 的 lifecycle 更新时，只更新原 cell 状态、时间和详情；不要追加一个同 id 的重复 cell。
- 不同 id 的 terminal event 到达晚于后续 item 时，应保持后端顺序或使用后端提供的 canonical sequence；客户端不凭时间戳猜测重排。
- compact replacement history 内部也遵守同一 id 与顺序规则，不因为 item 是 archive、terminal 或 fallback 就丢弃。

## root-worker 侧 UX 回归风险

- 如果客户端把 EventMsg 直接当渲染模型，可能绕过现有 `ConversationEntry` 归一化，导致搜索、定位、高亮、虚拟列表高度和 RightPanel 跳转失效。
- 如果迁移时用内容相等做合并，连续 command notification、goal event、workflow progress 或 child completion 会被误删，用户会以为某些等待或完成从未发生。
- 如果 `thread/read` 在 live thread 上回填 snapshot，可能打乱正在显示的 live item，造成闪烁、重复或历史倒序。
- 如果 wait timeout 显示 hard cap，用户会误判 command_wait / wait_agent 的短窗口行为。
- 如果 command_write_stdin 展示完整输入，可能把用户输入的 token、路径或 secret 暴露到默认历史。
- 如果 Agent Tree 从 conversation/EventMsg 推导状态，运行中、等待子 agent、等待 event tool 的状态会和 canonical `ThreadStatus` 分叉。
- 如果 typed fallback 缺失，replacement history 可能重新露出 raw function call/output JSON，降低 compact 后历史可读性。
- 如果 goal/workflow/event-command 被渲染成普通 agent message，用户会无法区分模型叙述和系统事实记录。

## 开发 Handoff

实现边界：

- app-server 可以将 EventMsg 作为 display source，但必须在 app-server/protocol 边界 canonicalize 为 typed `ThreadItem` payload，再交给 root-worker。
- root-worker 不新增 EventMsg raw parser；只扩展或复用 `ThreadItem` mapper、`buildConversationItemEntries`、`buildConversationCells`、RightPanel indexing 和 search/indexing。
- snapshot normalization、live cache merge、pending/live 合并继续只按 `ThreadItem.id` 判断同一个 item。
- command、collab、goal、workflow、event-command 的 UI 文案由 typed 字段生成，不读 raw text。
- fallback event 也必须由 app-server/protocol projector 生成 typed `ThreadItem` 或 typed fallback lifecycle payload。root-worker 只能消费该 typed fallback；不得在客户端从 raw function output、EventMsg debug text、legacy JSON envelope 或 assistant text 自行解析生成 fallback。

建议验收：

- live item 与 cold `thread/read` 对同一 typed history 渲染一致。
- 同 id started/completed 更新只产生一个 cell；不同 id 同文案事件保留多条。
- command notification、command_wait、command_write_stdin、wait_agent、child completion、goal complete、workflow terminal、event-command failed 均不显示 raw JSON。
- RightPanel row 点击定位使用 item id，定位后不抢 composer focus。
- Agent Tree 状态只随 `thread/status/changed` 更新，不随 conversation item text 改变。
- compact replacement history 不显示等待类工具 protocol start 行或 JSON output；缺 typed item 时显示低权重语义 fallback。
- root-worker 代码中不存在 EventMsg raw parser 或基于 raw text/JSON 的 display fallback。
- schedule subscribe、同一 recurring subscription 的多次 fired、unsubscribe 和 failed 均保留独立 typed conversation entries，且 RightPanel 定位使用 item id。

## 未决问题

- EventMsg 到 `ThreadItem` 的 canonical 字段命名需要由 app-server/protocol owner 定稿；UI 只要求字段具备稳定 id、kind、status、timestamps、target item id 和结构化 reason。
- workflow terminal progress 是更新同一 progress item 还是追加 terminal item，需要实现侧选择一种稳定策略，并在 `ThreadItem.id` 规则中固定。
- command_write_stdin 的异常状态如果后端暂不提供 typed result，UI 只能展示 `Sent`，不能推断 `Rejected` 或 `Command unavailable`。

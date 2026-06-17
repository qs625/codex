# Root Worker Client UI Brief

## APP 基线

root-worker prototype 是面向 my-codex 多 agent 调试和协作执行的桌面客户端。核心用户是高频使用 thread、subagent、command session、workflow 和 typed app-server v2 事件的工程师与 PM agent 调试者。用户需要快速判断当前 thread 在做什么、是否等待外部事件、下一步该输入什么，以及如何从一个 root session 切换到另一个 agent/thread。

基线信息架构：

- 左侧：Agent Tree，承担 root/subagent 层级导航、创建 root、thread 状态扫描。
- 中间：thread conversation，显示 selected thread 的 typed `ThreadItem` 投影内容、顶部 thread metadata、底部 composer。
- 右侧：Todo Board / Thread Analysis / File Preview 等辅助面板，通过最右侧 rail 切换。
- Composer：支持普通消息、附件、图片、voice、运行配置和 slash menu。当前 slash menu 已有 Commands / Skills 分组，内置命令只有 `/clear`。

设计原则：

- Conversation 是事实记录：所有可回溯内容以 typed `ThreadItem` / typed conversation event 为来源，不从 raw marker 或 message text 反解。
- Right Panel 是实时索引：只显示需要用户关注或可快速跳转的 live/recent 状态，不替代 conversation 详情。
- 命令信息分层：command cell 解释一次 command session 的完整生命周期；notification event 解释后续 output/exit 通知；Right Panel 只给定位和摘要。
- 不打断输入：右侧点击定位只滚动和高亮 conversation cell，不改变 composer draft，不抢输入焦点。

既有设计基线：

- Command Session UI details 已定义 command cell、notification event、Live Commands 和点击定位规则，baseline 见 [baseline-command-session-2026-06-14.png](/Users/bytedance/Projects/my-codex/.worktrees/slash-goal-display/ui-design/root-worker-client/assets/baseline-command-session-2026-06-14.png)。
- Composer Slash 菜单已定义 Commands / Skills 分组、`/clear` registry、键盘行为和 Skill chip 保留规则，feature handoff 见 [slash-menu.md](/Users/bytedance/Projects/my-codex/.worktrees/slash-goal-display/ui-design/root-worker-client/features/slash-menu.md)。

## 当前 Feature

本次增量设计覆盖 slash command 与 thread goal 可视化：

1. slash command 列表显示 runtime 内置命令，例如 `/goal cancel`；`/init` 通过 system skill 出现。
2. thread 页面显示当前 thread 的 goal 状态和 goal 内容。
3. 用户可通过 slash command 取消当前 thread goal。

### 2026-06-16 增量：Goal Command Actions

本次轻量 handoff 在既有 goal 展示与 `/goal cancel` 基线上，补齐 goal slash command 的动作族：

1. `/goal <objective>`：创建或更新当前 thread 的 active goal。
2. `/goal pause`：暂停当前 active goal。
3. `/goal resume`：恢复 paused goal。
4. `/goal cancel` / `/goal clear`：取消或清除当前 goal；`/cancel-goal` 仅作为兼容别名，不进入 slash menu 主展示。

交互目标是让用户在 composer 内完成低频 goal 管理，同时保持主界面高频扫描能力：GoalStrip 显示当前状态和关键操作，Thread Analysis 展示完整内容、预算和最近 action 反馈。

### 2026-06-16 增量：Goal ThreadItem Display

本次 handoff 补齐模型工具创建、读取、完成 goal 后在 conversation 中的 typed lifecycle 展示。它不是替代 GoalStrip 或 RightPanel 的 thread goal state，而是让用户能在时间线里回看“goal 什么时候被模型设置、读取、更新或完成”。

目标：

1. `create_goal` / `/goal <objective>` 成功后，conversation 追加 typed goal lifecycle item，显示 `Goal created` 或 `Goal updated`、目标摘要和时间。
2. `get_goal` 只在工具调用被投影为可见 `ThreadItem` 时显示轻量 `Goal checked` 事件；不因为普通内部查询制造噪音。
3. `update_goal(status=complete)` 成功后，conversation 追加 `Goal complete` item，并和 GoalStrip 收起/RightPanel terminal event 保持一致。
4. 历史 snapshot、live item、compact replacement history 中的 goal item 都按 `ThreadItem.id` 保留，不按 goal 文本去重。

### 2026-06-16 增量：EventMsg Display Source Migration

本次 handoff 覆盖 EventMsg 成为 app-server / root-worker UI display source 后的 UX 不变量。目标是后端 display source 可替换，但 root-worker 用户可见 conversation、live update、thread/read、replacement history、RightPanel 定位和 Agent Tree 状态保持不变。

关键原则：

1. root-worker 仍消费 typed `ThreadItem` / typed lifecycle payload，不新增 EventMsg raw parser。
2. `ThreadItem.id` 仍是 conversation 合并、去重、定位、live 更新和 replacement history 保留的唯一键。
3. command、collab/child completion、goal、workflow、event-command 等系统事实继续作为 typed event 展示，不退化成普通 agent message 或 raw JSON。
4. 本次为文字级迁移 handoff，不改变 UI 视觉形态，因此不新增 baseline screenshot 或原型资产；如实现改变布局或组件视觉，需重新获取完整 Electron baseline。

### 2026-06-17 增量：Workflow Progress Display

本次 handoff 补齐 Dynamic Workflow 启动后在 root-worker conversation 中的图与进度展示。它承接 slash workflow discovery：slash menu 只负责生成可编辑草稿，模型在当前 turn 中调用 `workflow_start` / `workflow_resume` 后，进度必须由 typed display path 进入线程时间线。

目标：

1. 当后端发出 `EventMsg::WorkflowRunProgressCompleted -> ThreadItem::WorkflowRunProgress` 时，conversation 插入低噪音但可扫描的 workflow progress cell。
2. cell 以 static graph 的 stage 列表表达 workflow 结构，以状态 badge / progress rail 表达当前 stage、完成、失败或取消。
3. root-worker 只消费 typed `ThreadItem` / v2 payload；不得从 raw marker、assistant JSON、workflow tool output 或 legacy envelope 反解进度。
4. 如果 typed `ThreadItem` 暂缺 workflow progress variant，协议层应补齐 typed payload；UI 只接受 typed fallback，不新增 raw parser。
5. workflow 创建的 thread / agent 通过 thread metadata workflow binding 显示轻量所属 badge；progress cell 展示 run 进展，badge 只展示所属关系。

本次属于现有 conversation event 体系的小幅视觉增量，已重新获取完整 Electron baseline：

- [baseline-workflow-progress-2026-06-17.png](/Users/bytedance/Projects/my-codex/.worktrees/workflow-slash-commands-client-display/ui-design/root-worker-client/assets/baseline-workflow-progress-2026-06-17.png)
- 原型图：[workflow-progress-prototype.svg](/Users/bytedance/Projects/my-codex/.worktrees/workflow-slash-commands-client-display/ui-design/root-worker-client/assets/workflow-progress-prototype.svg)

## 目标用户

- 角色：使用 Goal/Go 持续推进长任务的 owner、PM agent、调试 root-worker prototype 的工程师。
- 频率：创建或取消 goal 是低频动作；查看 goal 状态是高频扫描动作。
- 设备：桌面 Electron 为主，窄宽度 renderer 需要保持可读。
- 专业程度：熟悉 thread 状态、typed item、slash command；不应要求用户理解 raw marker 或历史 envelope。

## 范围

涉及：

- Composer slash menu 的 command 列表、搜索、键盘选择和 disabled/empty 状态。
- Selected thread header 下方的 Goal Strip，用于显示 goal 状态、内容摘要、预算和取消入口。
- Thread Analysis 面板中的 Goal Detail 区块，用于展示更完整的 goal 内容、预算、最近事件和取消反馈。
- `/goal cancel`、无 goal、active、paused、complete、budget limited、取消中、取消失败等状态。
- EventMsg display source 迁移后的 conversation/live/thread-read/replacement history/RightPanel/Agent Tree 用户可见不变量。

不涉及：

- Goal runtime 语义、后端 API、typed payload 的字段命名最终决定。
- 从 agent 文本、raw marker、legacy JSON envelope 解析 goal 内容。
- 重做 conversation cell 布局、Agent Tree 状态模型或右侧面板整体视觉。

## 数据与实现约束

- Goal 显示必须消费 typed state/API：优先使用后端 canonical goal state / v2 payload；如以 conversation 事件展示，必须经 typed `EventMsg -> ThreadItem` 投影，迁移期可兼容 `ResponseItem::ThreadGoalUpdate`。
- Slash command 执行可以走现有 composer command 分发，但 command 的可用性和执行结果不能通过解析 assistant 文本判断。
- Thread running/waiting/idle 继续消费 canonical `ThreadStatus` / `thread/status/changed`。
- Cancel 成功或失败需要产生可追踪反馈：优先为 composer 局部状态 + typed goal event；不可只依赖 toast 或 agent message 文本。

## Baseline 截图

本任务涉及现有 root-worker prototype 客户端 UI，已使用 `$root-worker-playwright-debug` 的完整 Electron smoke 脚本获取 baseline：

- [baseline-slash-goal-display.png](/Users/bytedance/Projects/my-codex/.worktrees/slash-goal-display/ui-design/root-worker-client/assets/baseline-slash-goal-display.png)
- [baseline-goal-command-actions-2026-06-16.png](/Users/bytedance/Projects/my-codex/.worktrees/goal-command-actions/ui-design/root-worker-client/assets/baseline-goal-command-actions-2026-06-16.png)
- [baseline-goal-threaditem-display-2026-06-16.png](/Users/bytedance/Projects/my-codex/.worktrees/goal-threaditem-display/ui-design/root-worker-client/assets/baseline-goal-threaditem-display-2026-06-16.png)

截图环境使用 skill 默认隔离目录：

- `CODEX_HOME=/tmp/my-codex-root-worker-debug/codex-home`
- `ROOT_WORKER_WORKSPACE=/tmp/my-codex-root-worker-debug/workspace`

限制：截图顶部显示 app-server bootstrap 失败 `app-server exited (127 / null)`，因此未捕获真实已有 thread 内容；但 Electron shell、三栏布局、composer、右侧 rail、empty state 和 slash command 所在位置可作为本次 UI 基线。

2026-06-16 截图同样使用 `$root-worker-playwright-debug` 完整 Electron smoke 获取，`window.codexDesktop` 可用；由于本地 app-server binary 不可用，截图只作为真实 Electron shell 和布局基线，不作为 goal 数据状态截图。

本次 Goal ThreadItem Display baseline 同样使用 `$root-worker-playwright-debug` 的完整 Electron smoke 脚本获取，`window.codexDesktop=true`。本地 `codex-app-server` 仍退出 `127 / null`，因此截图只作为 Electron shell、三栏布局、conversation 空态和 composer 位置基线；goal lifecycle 视觉由原型图表达。

## 验收标准

- `/` 打开 slash menu 时，内置 command 以 Commands 分组显示，`/goal cancel` 能被搜索、键盘选中、点击执行；`/init` 作为 system skill 出现在 Skills 分组。
- 无 goal 时，thread header 不制造显著噪音；Thread Analysis 可显示简短空态。
- Active goal 时，中间 conversation 顶部出现 Goal Strip，能在一眼内看到状态、目标摘要和预算/continuation 信息。
- Goal 内容较长时，header strip 只展示一到两行摘要，完整内容进入右侧 Thread Analysis。
- Cancel 入口在 `/goal cancel` 和 Goal Strip 内均可达；取消中禁用重复触发；取消成功、失败、无 goal 三类反馈明确。
- 视觉保持当前 root-worker 工程工具风格：浅色、低装饰、高信息密度、8px 左右圆角、清晰 focus ring。
- 设计进入开发前完成独立 UI/UE review，并在 `05-review.md` 记录结论。
- 模型通过 goal 工具创建、更新、完成 goal 时，conversation 至少生成一个 typed `ConversationEntry`，视觉上与普通 agent message 分离，并可被搜索、定位和历史回放保留。
- EventMsg 迁移后 root-worker 不新增 raw parser；缺 typed item 的 fallback 也必须由 app-server/protocol projector 产出 typed payload。

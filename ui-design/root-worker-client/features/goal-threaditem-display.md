# Feature: Goal ThreadItem Display

## 目标

让 root-worker prototype 在 conversation 中展示模型 goal 工具产生的 typed lifecycle item。用户需要能回看 goal 何时被创建、更新、查询、暂停、恢复、取消、完成或因预算停止，同时继续用 GoalStrip 扫描当前状态，用 RightPanel 查看完整详情。

本 feature 不改变 goal runtime 语义，不解析 assistant 文本、raw marker、legacy envelope 或 `<goal_context>`。所有展示必须来自 app-server v2 typed `ThreadItem` / typed payload。

## 用户可见成功标准

- 模型调用 `create_goal` 并成功后，conversation 出现一条 `Goal created` event cell，包含状态 badge、目标摘要和时间；如果已有 goal 被替换或更新，标题为 `Goal updated`。
- 模型调用 `get_goal` 时，默认不产生高噪音历史项；只有后端明确投影了 typed `goalChecked` / `goalRead` item 时，conversation 显示单行 `Goal checked`，内容不重复展开完整 objective。
- 模型调用 `update_goal(status=complete)` 并成功后，conversation 出现 `Goal complete` event cell，用户能清楚知道目标已经结束，而不是只看到 GoalStrip 消失。
- pause/resume/cancel/budget limited 等状态如果已有 typed lifecycle item，也以同一组件展示；失败状态显示结构化原因，例如 `Could not update goal: <reason>`。
- live 和 snapshot 路径一致：同一个 `ThreadItem.id` 的 started/completed 更新合并为同一 cell；不同 id 即使文本相同也保留为不同事件。

## 视觉形态

Conversation 中新增轻量 `GoalLifecycleEventCell`，继承现有 `.event-row` 的低噪音时间线形态，但需要比普通 share-icon event 更可辨认。

结构：

- 左侧图标：使用现有 icon 体系中的 target/flag/check/circle 类图标；若当前库没有目标图标，优先新增简洁 `TargetIcon`，不要使用彩色插画。
- 主体 pill：一行标题 + 状态 badge + 时间；第二行可选 objective 摘要，最多两行截断。
- Meta：可选显示 `Tool: create_goal`、`Budget limited`、`from model tool` 等短信息；不显示 raw JSON。

状态文案：

- `Goal created`
- `Goal updated`
- `Goal checked`
- `Goal paused`
- `Goal resumed`
- `Goal cancelled`
- `Goal complete`
- `Goal budget limited`
- `Goal action failed`

色彩：

- active/created/updated：低饱和 amber，和现有 search/goal accent 保持同族但面积更小。
- complete：green success badge，只用于 badge 和左侧小图标，不整条染色。
- paused/checked：neutral gray badge。
- budget limited：warning amber badge，文案必须出现 `Budget limited`，不能只靠颜色。
- failed：red text/badge，错误原因放在第二行或 details，不把整条 cell 变成大红背景。

响应式与长文本：

- 第一行使用三段布局：title、badge、time。badge 固定不压缩；title 使用 `min-width: 0` 和单行截断；time 优先保留在行尾。
- 中等宽度下，如果 title + badge + time 空间不足，time 下移到第二行 meta；title 仍单行截断，badge 保持完整文本。
- 窄宽度下，time 可隐藏到 `aria-label` / tooltip，并在详情或 RightPanel recent event 中保留；badge 不隐藏、不只用颜色。
- objective preview 使用 `min-width: 0`、两行 clamp 和自然换行；失败原因同样最多两行，完整 reason 进入 details/RightPanel。
- 圆角跟随现有 event/tool row 体系，不做独立大 card 视觉；原型只表达信息层级。

## 状态与历史行为

空态：

- 无 goal item 时 conversation 不显示占位；空态继续由 GoalStrip/RightPanel 处理。
- app-server bootstrap 失败时不制造 goal placeholder；全局错误已足够。

重复与去重：

- `ThreadItem.id` 是唯一合并键。不得按 `objective`、`status`、`toolName`、`text` 或时间窗口合并。
- 同一 item 的 started/completed 更新可以更新 status、completed time、error reason。
- 连续 goal lifecycle items 不和普通 `event` 自动视觉合并成一个长 cell；每个 typed item 至少一个 `ConversationEntry`，可以在视觉上保持相邻但不丢 entry。

历史 snapshot：

- cold start `thread/read` 加载到的 goal lifecycle items 直接按 typed item 投影为 conversation entry。
- live cache 已有 thread 时，不能用 snapshot 回填破坏或重排已经接收的 goal items。
- compact replacement history 中的 goal lifecycle item 仍作为 archive/compact 内部 entry 展示，不因它是 terminal state 而丢弃。

## 与 GoalStrip / RightPanel 的关系

- GoalStrip：只回答“当前 thread 现在的 goal 是什么、还能做什么”。它可以因 complete/cancelled 收起或显示短暂 terminal feedback。
- Conversation goal item：回答“这个 goal lifecycle 事件什么时候发生、由哪个 typed item 记录”。它是审计线索和搜索/定位对象。
- RightPanel Goal Detail：回答“完整 objective、预算、最近 lifecycle event 和可执行 action”。最近 event 可以引用 conversation item id，用于点击定位。

避免重复的规则：

- Conversation cell 只显示摘要，不重复展示完整 objective、完整预算表和所有 action button。
- RightPanel 的 recent event 可以显示与 conversation 相同的短标题，但不要再渲染一整条 conversation cell。
- GoalStrip 不展示历史事件列表，只展示当前状态和最多一个最近 action error。

## UE 流程 delta

### create_goal 成功

1. 模型调用 `create_goal`。
2. 后端写入 typed `ResponseItem` 并投影为 `ThreadItem.goalLifecycle` 或等价 goal item。
3. live `item/completed` 到达后，conversation 插入 `Goal created` cell。
4. GoalStrip 同步显示 active objective；RightPanel Goal Detail 的 Recent event 指向这条 item。

### update_goal complete 成功

1. 模型调用 `update_goal(status=complete)`。
2. conversation 插入 `Goal complete` cell，显示目标摘要和完成时间。
3. GoalStrip 收起或显示短暂 `Goal complete` terminal feedback。
4. RightPanel 保留 terminal event，完整 goal 内容仍可查看。

### get_goal 可见化

1. 模型调用 `get_goal`。
2. 如果后端只将它作为工具内部上下文，不投影 conversation item，UI 不额外显示。
3. 如果后端投影 typed read/check item，conversation 显示 `Goal checked` 单行 event，默认不展开完整 objective。

## 开发 handoff

推荐新增或扩展的 typed item：

- `ThreadItem` 新 variant：`goalLifecycle`，或更具体的 `goalEvent`。字段建议：`id`、`action`、`status`、`objectivePreview`、`objective` 可选、`reason` 可选、`toolName`、`createdAtMs` / `completedAtMs`、`goalId` 可选、`budget` 可选。
- `ConversationEntry` 可复用 `kind: "event"`，新增可选 `eventCategory?: "goal" | ...` 更利于图标和样式选择；如果不扩类型，也可先用 `toolCategory` 不合适，避免把 goal lifecycle 伪装成 tool row。
- `ConversationCell` 不需要新增 kind；用 `event` cell 渲染，`EventRow` 根据 `eventCategory === "goal"` 切换图标、badge 和两行布局。

实现注意：

- `buildConversationItemEntries` 必须从 typed item 字段构造文案，不读取 `agentMessage.text`。
- goal item 进入 `buildConversationCells` 后不应和普通 event 丢失边界；若需要禁止连续 event 合并，保持现状即可，因为当前 event 本来不合并。
- 搜索、定位、高亮只基于生成后的 `ConversationEntry` / `ConversationCell`。
- `ThreadItem` snapshot normalization 和 live merge 继续按 `id` 判断同一 item；不要按 goal 文本去重。
- RightPanel 的 Recent event 使用 `item.id` 定位 conversation cell，不使用 objective 文本搜索。
- `apps/root-worker-prototype/src/lib/conversationVirtualization.ts` 需要同步 goal event 的高度估算，或确保测量后的高度稳定修正；两行 goal event、失败 reason、compact archive 内 goal event 都要覆盖定位和高亮验证。
- RightPanel Recent event 必须是可聚焦 button 或 link，Enter/Space 触发定位，保留清晰 focus ring；跳转后用非侵入式 `aria-live=polite` 状态提示已定位到对应 goal event，不抢 composer focus。

## 原型

- [goal-threaditem-display-prototype.svg](/Users/bytedance/Projects/my-codex/.worktrees/goal-threaditem-display/ui-design/root-worker-client/assets/goal-threaditem-display-prototype.svg)
- Baseline：[baseline-goal-threaditem-display-2026-06-16.png](/Users/bytedance/Projects/my-codex/.worktrees/goal-threaditem-display/ui-design/root-worker-client/assets/baseline-goal-threaditem-display-2026-06-16.png)

## 验收清单

- create/update/complete/cancel/budget-limited typed goal item 均有清晰 event cell。
- `get_goal` 不默认刷屏；只有 typed read/check item 才显示。
- 批量或 internal `get_goal` read 不生成 visible conversation item，除非后端显式标记为 user-visible typed item。
- GoalStrip、RightPanel、conversation 三者信息层级不重复。
- 历史 snapshot、live item、compact replacement history 均保留不同 id 的 goal items。
- 无 raw marker、assistant JSON、legacy envelope 解析路径。

## 剩余 UX 风险

- 如果后端把每次 internal `get_goal` 都投影为 visible item，conversation 会产生噪音；建议协议层区分 user-visible lifecycle 与 internal read。
- 如果 goal objective 很长，event cell 只显示 preview 可能让用户误以为内容被截断丢失；RightPanel 必须提供完整内容。
- 如果 terminal state 同时让 GoalStrip 消失，用户可能只看到状态条消失；conversation `Goal complete` item 是必要补偿，不能省略。

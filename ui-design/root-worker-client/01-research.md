# Research

## 是否做外部产品调研

本次不是新产品或大幅 UI 改造，而是在现有 root-worker prototype 的 thread/composer 框架内增加 goal 状态和 slash command。外部竞品调研不是阻塞项；设计主要继承当前客户端结构，并参考本仓库已有模式：

- Composer slash menu 已有 Commands / Skills 分组、listbox、键盘选择、empty state。
- Thread header 已显示 path、role、presence、运行配置，适合承载一行高优先级 thread-level 状态。
- Right Panel 已承担 Thread Analysis、Todo、Preview 等辅助信息，适合放 Goal Detail，不应把完整 goal 内容塞进 conversation 主流。
- Agent Tree 状态继续由 canonical `ThreadStatus` 驱动，本 feature 不新增 agent 状态推导。

既有轻量调研结论继续适用：

- Slash menu 采用 composer 锚定弹层，而不是全局 command palette，避免把日常消息输入切换成导航模式。
- 候选内容分为 `Commands` 和 `Skills`，用分组标题帮助用户判断选择后的后果。
- `Enter` 对内置命令是执行或提交该命令，对 Skill 走现有 chip/payload 行为；`Tab` 只做补全或选择，不自动发送普通消息。
- `/` 后继续输入即时过滤；没有匹配时保留 draft 并显示空态，不清空用户输入。
- Conversation 继续作为 canonical timeline，typed item id 是跨 conversation 与 RightPanel 的唯一可靠关联键。

## 现有 UI 观察

Baseline 截图：[baseline-slash-goal-display.png](/Users/bytedance/Projects/my-codex/.worktrees/slash-goal-display/ui-design/root-worker-client/assets/baseline-slash-goal-display.png)

当前页面可观察到：

- 三栏布局稳定：左侧 Agent Tree，中间 selected thread，右侧辅助 panel，最右 rail。
- Composer 在底部使用大输入框和图标按钮，slash menu 已在输入框下方浮现，视觉上属于 composer shell。
- Right Panel 默认是 Todo Board，Thread Analysis 在 rail 的 gear 入口，适合补充 Goal Detail。
- 空 thread 状态下中间内容非常空，Goal Strip 需要只在有 meaningful goal state 时出现，避免无 goal 噪音。

## 相关代码模式

只读查看的关键文件：

- `apps/root-worker-prototype/src/lib/slashMenu.ts`：内置 command 目前只有 `clear`；建议扩展 `ComposerSlashCommandId`，不要另起 command palette。
- `apps/root-worker-prototype/src/components/Panels.tsx`：slash menu 已有 `role=listbox`、Commands/Skills 分组、键盘交互和 empty state。
- `apps/root-worker-prototype/src/App.tsx`：`runComposerSlashCommand` 是 command 分发入口；`sendMessage` 已在发送前拦截 `/clear`。
- `apps/root-worker-prototype/src/types.ts`：`ThreadStatus` 只覆盖 active/idle/systemError/notLoaded；goal 应作为独立 typed thread goal state，不应混入 activeFlags。
- `apps/root-worker-prototype/src/components/RightPanel.tsx`：Thread Analysis 已通过 selected `thread` 派生面板内容，适合加入 Goal Detail。

## 设计推论

- `/goal cancel` 应被当作 command，而不是 skill。它改变 thread goal 生命周期，不是把 skill chip 加入 draft。
- `/init` 应来自 system skill discovery，不进入 root-worker builtin command registry。
- `/goal cancel` 在无 active goal 时仍可显示，但应 disabled 或执行后给 “No active goal” 的轻量反馈；推荐 disabled，因为 command list 能提前解释不可用原因。
- Goal 状态应有两个层级：header strip 用于高频扫描，Right Panel detail 用于完整内容和最近事件。
- 取消反馈应尽量靠近触发点：从 slash menu 执行后 composer status 显示短反馈；从 Goal Strip button 触发后 button 进入 cancelling 状态，同时 typed goal event 进入 history/detail。

## 2026-06-16 Goal Command Actions 轻量调研

本次不新增外部竞品调研，原因是范围是现有 composer command grammar 和 GoalStrip/RightPanel action 的增量，不是新页面或大幅视觉改造。采用仓库内既有模式即可覆盖主要风险：

- Slash menu 主展示继续使用一条 `/goal <subcommand>` command family，避免扩散 `/goal-cancel`、`/goal-pause` 这类顶层主命令。
- `/cancel-goal` 只作为输入兼容别名：可被 parser 识别，可作为搜索 alias 命中 `/goal cancel`，但菜单 label 不展示为主命令。
- `/goal <objective>` 不应在用户只输入 `/goal` 时立即执行；它是带参数 command，需要在 composer 中继续输入 objective 后通过 send 执行。
- `pause`、`resume`、`cancel|clear` 是有副作用的无参数 action command。为降低误触，slash menu 的 Enter/鼠标点击只补全完整 token 并保持 composer focus；用户再按 Enter 执行该完整 command。
- Goal action 可用性必须由 typed goal state 和本地 action pending state 派生：active 可 pause/cancel/clear，paused 可 resume/cancel/clear，complete/budgetLimited 是否可 cancel 由后端能力决定，缺失 goal 时 action row 显示 disabled reason。

只读查看发现当前实现已具备 `ThreadGoal`、GoalStrip、GoalDetailPanel、`thread/goal/updated` / `thread/goal/cleared` notification 和 `/goal cancel` / `/cancel-goal` 拦截。缺口主要是 command id 粒度、draft parser、二级 slash query、pause/resume/create/update action 反馈，以及 menu 文案从单个 command 扩展为 command family。

## 2026-06-16 Goal ThreadItem Display 调研

本次仍不做外部竞品调研，原因是问题集中在 root-worker 自身的 typed timeline 语义：goal lifecycle item 应如何进入现有 `ThreadItem -> ConversationEntry -> ConversationCell` 展示链路。外部产品的任务/目标 UI 对本问题帮助有限，反而容易引入和 app-server v2 typed display 边界不一致的模式。

只读观察：

- `apps/root-worker-prototype/src/types.ts` 中 `ConversationEntry.kind` 目前包含 `message`、`event`、`tool`、`compact`、`archive`；goal lifecycle 更接近 `event`，不是普通 agent message，也不是 command/tool row。
- `apps/root-worker-prototype/src/lib/conversation.ts` 的 `buildConversationItemEntries` 已经按 typed `ThreadItem.type` 分支生成 entry；新增 goal 展示应在这里接 typed item，不应从 `agentMessage.text` 反解。
- `buildConversationCells` 只合并 agent message 和同类 tool；event 默认不合并，适合保留每个 goal lifecycle item 的独立边界。
- `EventRow` 当前统一使用 `ShareIcon` 和单行 pill；goal event 需要在不扩展大面积视觉的前提下增加 event category、状态 badge 和第二行摘要。
- 当前搜索、虚拟列表和 archived history 都基于 `ConversationEntry` / `ConversationCell`；goal item 只要进入该链路，就能自然参与搜索、定位和 compact archive。

设计推论：

- goal lifecycle 最小可复用 event cell，而不是新增完整 card。它应该像 command notification 一样低噪音，但有更明确的 goal 图标和状态 badge。
- `get_goal` 是潜在噪音源。默认不应因为每次内部读取都显示历史项；只有后端明确投影 user-visible typed read/check item 时才展示。
- GoalStrip 和 RightPanel 已承担当前状态和详情，不应在 conversation 再重复完整 objective、预算表和 action button。
- 如果后端只提供 `builtinToolCall` / `eventDrivenToolCall` 的泛型工具项，短期可以展示工具 row；但最终 goal lifecycle 应有专门 typed item 或足够明确的 typed payload，避免 UI 从工具名和 output JSON 中猜语义。

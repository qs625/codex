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

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

不涉及：

- Goal runtime 语义、后端 API、typed payload 的字段命名最终决定。
- 从 agent 文本、raw marker、legacy JSON envelope 解析 goal 内容。
- 重做 conversation cell 布局、Agent Tree 状态模型或右侧面板整体视觉。

## 数据与实现约束

- Goal 显示必须消费 typed state/API：优先使用后端 canonical goal state / v2 payload；如以 conversation 事件展示，必须经 typed `ResponseItem -> ThreadItem` 投影。
- Slash command 执行可以走现有 composer command 分发，但 command 的可用性和执行结果不能通过解析 assistant 文本判断。
- Thread running/waiting/idle 继续消费 canonical `ThreadStatus` / `thread/status/changed`。
- Cancel 成功或失败需要产生可追踪反馈：优先为 composer 局部状态 + typed goal event；不可只依赖 toast 或 agent message 文本。

## Baseline 截图

本任务涉及现有 root-worker prototype 客户端 UI，已使用 `$root-worker-playwright-debug` 的完整 Electron smoke 脚本获取 baseline：

- [baseline-slash-goal-display.png](/Users/bytedance/Projects/my-codex/.worktrees/slash-goal-display/ui-design/root-worker-client/assets/baseline-slash-goal-display.png)

截图环境使用 skill 默认隔离目录：

- `CODEX_HOME=/tmp/my-codex-root-worker-debug/codex-home`
- `ROOT_WORKER_WORKSPACE=/tmp/my-codex-root-worker-debug/workspace`

限制：截图顶部显示 app-server bootstrap 失败 `app-server exited (127 / null)`，因此未捕获真实已有 thread 内容；但 Electron shell、三栏布局、composer、右侧 rail、empty state 和 slash command 所在位置可作为本次 UI 基线。

## 验收标准

- `/` 打开 slash menu 时，内置 command 以 Commands 分组显示，`/goal cancel` 能被搜索、键盘选中、点击执行；`/init` 作为 system skill 出现在 Skills 分组。
- 无 goal 时，thread header 不制造显著噪音；Thread Analysis 可显示简短空态。
- Active goal 时，中间 conversation 顶部出现 Goal Strip，能在一眼内看到状态、目标摘要和预算/continuation 信息。
- Goal 内容较长时，header strip 只展示一到两行摘要，完整内容进入右侧 Thread Analysis。
- Cancel 入口在 `/goal cancel` 和 Goal Strip 内均可达；取消中禁用重复触发；取消成功、失败、无 goal 三类反馈明确。
- 视觉保持当前 root-worker 工程工具风格：浅色、低装饰、高信息密度、8px 左右圆角、清晰 focus ring。
- 设计进入开发前完成独立 UI/UE review，并在 `05-review.md` 记录结论。

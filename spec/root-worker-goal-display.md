# Root-worker Thread Goal 展示与取消

## Brief

用户：root-worker prototype 的桌面客户端用户。

能力：用户选中 thread 后，可以在 thread 页面看到当前 persisted goal 的状态和内容，并能通过 `/goal cancel` 或页面按钮人工取消当前 goal。

成功标准：

- 选中 thread 时通过 `thread/goal/get` 读取当前 goal。
- 收到 `thread/goal/updated` / `thread/goal/cleared` typed notification 后，header strip 和 Thread Analysis detail 同步更新。
- `/goal cancel` 和 `/cancel-goal` 不作为普通 user message 发送给模型，而是调用 `thread/goal/clear`。
- 取消 pending、无 goal、失败都显示结构化反馈，不解析 assistant 文本、raw marker 或 legacy envelope。
- `goal` 不混入 `ThreadStatus.activeFlags`；active state 继续只表达 thread 是否运行或等待外部输入。

非目标：

- 不新增 goal 创建 API；模型工具和 app-server 已有 `create_goal` / `thread/goal/set` 负责创建或更新。
- 不用 `/init` 设置 goal；`/init` 是 system skill，不是 runtime goal command。
- 不改变 app-server v2 协议字段。

## 技术设计

- Electron main 暴露 `getThreadGoal(threadId)` 和 `clearThreadGoal(threadId)`，内部调用 app-server v2 `thread/goal/get` 与 `thread/goal/clear`。
- Electron notification normalization 只归一化 typed `ThreadGoal` payload，不解析 conversation 文本。
- `App.tsx` 维护 `goalsByThreadId`、`goalCancelingThreadIds`、`goalActionErrorsByThreadId`。
- `ConversationPanel` 渲染 `GoalStrip`：无 goal 且无错误时不占位；有 goal 时显示 status、objective、token usage 和 cancel。
- `RightPanel` 的 Thread Analysis 渲染 `GoalDetailPanel`：显示完整 objective、token budget、elapsed time 和同一个 cancel action。
- `composerDraft.ts` 识别手动 `/goal cancel` 与 `/cancel-goal`，要求没有 image attachment 和 Skill chip，避免误拦截普通带上下文输入。

## 风险

- `thread/goal/get` 失败时当前客户端把 goal 视作不可用并隐藏 strip，避免误展示 stale 状态；详细错误仍由全局 app-server 错误或 cancel action error 暴露。
- 如果 clear RPC 返回 `cleared: false`，UI 显示 `No active goal to cancel.`，并清空当前本地 goal 视图以跟随后端状态。
- `complete` goal 仍按后端返回展示；是否长期保留 complete goal 由后端状态生命周期决定。

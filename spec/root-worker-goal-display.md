# Root-worker Thread Goal 展示与操作

## Brief

用户：root-worker prototype 的桌面客户端用户。

能力：用户选中 thread 后，可以在 thread 页面看到当前 persisted goal 的状态和内容，并能通过 `/goal <objective>`、`/goal pause`、`/goal resume`、`/goal cancel|clear` 或页面按钮创建、更新、暂停、恢复、取消当前 goal。

成功标准：

- 选中 thread 时通过 `thread/goal/get` 读取当前 goal。
- 收到 `thread/goal/updated` / `thread/goal/cleared` typed notification 后，header strip 和 Thread Analysis detail 同步更新。
- `/goal <objective>` 调用 `thread/goal/set` 创建或更新 active goal；精确 `/goal pause` / `/goal resume` 调用 `thread/goal/set` 更新 status；精确 `/goal cancel` / `/goal clear` / `/cancel-goal` 调用 `thread/goal/clear`。
- `/goal pause this migration` 这类非精确动作输入按 objective 处理。
- 空 `/goal` / `/goal ` 显示 `Enter a goal objective.`，不发送给模型。
- goal action pending、无 goal、失败都显示结构化反馈，不解析 assistant 文本、raw marker 或 legacy envelope。
- 模型调用 `create_goal` / `update_goal` 改变 goal 时，conversation 中出现 typed goal item，明确说明目标已创建、已完成或已更新；该 item 来源于 typed `EventMsg -> ThreadItem` display lifecycle，迁移期可通过 `ResponseItem::ThreadGoalUpdate -> EventMsg::ResponseItemCompleted` 兼容路径生成，不从 tool output JSON、assistant 文本或 legacy marker 反解。客户端 goal API 仍以 `thread/goal/updated` / `thread/goal/cleared` 更新当前状态，未来如需要会话流事件也必须复用同一 typed item。
- `goal` 不混入 `ThreadStatus.activeFlags`；active state 继续只表达 thread 是否运行或等待外部输入。

非目标：

- 不新增 app-server goal API；模型工具和 app-server 已有 `create_goal` / `thread/goal/set` 负责创建或更新。
- 不用 `/init` 设置 goal；`/init` 是 system skill，不是 runtime goal command。
- 不改变 thread goal state API 的语义。

## 技术设计

- Electron main 暴露 `getThreadGoal(threadId)`、`setThreadGoal(payload)` 和 `clearThreadGoal(threadId)`，内部调用 app-server v2 `thread/goal/get`、`thread/goal/set` 与 `thread/goal/clear`。
- Electron notification normalization 只归一化 typed `ThreadGoal` payload，不解析 conversation 文本。
- `App.tsx` 维护 `goalsByThreadId`、`goalActionByThreadId`、`goalActionErrorsByThreadId`。
- `ConversationPanel` 渲染 `GoalStrip`：无 goal 且无错误时不占位；有 goal 时显示 status、objective、token usage，active 显示 Pause，paused 显示 Resume，并保留 Cancel。
- `RightPanel` 的 Thread Analysis 渲染 `GoalDetailPanel`：显示完整 objective、token budget、elapsed time 和同一组 Pause/Resume/Cancel action。
- `composerDraft.ts` 识别手动 `/goal <objective>`、精确 `/goal pause|resume|cancel|clear` 与 `/cancel-goal`，要求没有 image attachment 和 Skill chip，避免误拦截普通带上下文输入。
- core 在模型工具触发的 `create_thread_goal` / `set_thread_goal` 成功持久化 goal 后，迁移期写入 `ResponseItem::ThreadGoalUpdate` 并通过 `record_model_items_and_emit_display_events` 发送 typed display lifecycle，同时继续发送现有 `thread/goal/updated` state notification；后续新增 goal 展示语义应迁向 dedicated `EventMsg` variant。
- `app-server-protocol` 在 shared projector 中把 `ResponseItem::ThreadGoalUpdate` 映射为 `ThreadItem::ThreadGoalUpdate`，历史重建和 live notification 复用同一路径。
- root-worker 的 `ThreadItem` 类型和 `buildConversationItemEntries` 消费 `threadGoalUpdate`，生成 `toolCategory: "goal"` 的系统 tool entry；展示为会话事件，不和连续 agent message 合并，也不参与 `ThreadItem.id` 去重以外的合并。

## 风险

- `thread/goal/get` 失败时当前客户端把 goal 视作不可用并隐藏 strip，避免误展示 stale 状态；详细错误仍由全局 app-server 错误或 cancel action error 暴露。
- 如果 clear RPC 返回 `cleared: false`，UI 显示 `No active goal to cancel.`，并清空当前本地 goal 视图以跟随后端状态。
- `complete` goal 仍按后端返回展示；是否长期保留 complete goal 由后端状态生命周期决定。
- 旧 rollout 没有 `ThreadGoalUpdate` 历史项时，只会显示现有 state strip/detail；新增历史项从本改动之后的 goal mutation 开始可见。
- 客户端 `/goal` 直接调用 app-server goal API 的 idle 更新当前不创建 conversation item；如果后续产品要求记录用户发起的 goal lifecycle 历史，需要在 app-server/client mutation 路径补同一 `ResponseItem::ThreadGoalUpdate`，不能手写普通 agent message。

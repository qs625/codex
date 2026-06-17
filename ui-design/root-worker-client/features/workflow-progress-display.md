# Workflow Progress Display

## 目标

让用户在 root-worker conversation 中看到 Dynamic Workflow 启动后的 static graph 和运行进度，并能在 Agent Tree / conversation header 轻量识别当前 thread 属于哪个 workflow run。展示只基于 typed `ThreadItem::WorkflowRunProgress` 和 thread metadata workflow binding。

## 范围

涉及：

- Conversation 中新增或扩展 `WorkflowProgressCell`。
- Agent Tree / conversation header 中新增轻量 `WorkflowThreadBadge`。
- Workflow typed payload 字段、状态文案、空/失败状态。
- Slash workflow 候选与 progress 出现之间的交互关系。
- 搜索、定位、compact history、窄屏和可访问性要求。

不涉及：

- 客户端直接调用 `workflow/start`。
- 新增 Workflow 控制面板或 abort/resume 按钮。
- 把 Agent Tree 改成 workflow graph。
- 从 raw marker、assistant JSON、workflow tool output、legacy envelope 解析图、进度或 thread 所属关系。

## Baseline 与原型

- Electron baseline：[baseline-workflow-progress-2026-06-17.png](/Users/bytedance/Projects/my-codex/.worktrees/workflow-slash-commands-client-display/ui-design/root-worker-client/assets/baseline-workflow-progress-2026-06-17.png)
- 原型图：[workflow-progress-prototype.svg](/Users/bytedance/Projects/my-codex/.worktrees/workflow-slash-commands-client-display/ui-design/root-worker-client/assets/workflow-progress-prototype.svg)

截图使用 `$root-worker-playwright-debug` 的完整 Electron smoke 脚本获取，`window.codexDesktop=true`，使用隔离目录：

- `CODEX_HOME=/tmp/my-codex-root-worker-debug/codex-home`
- `ROOT_WORKER_WORKSPACE=/tmp/my-codex-root-worker-debug/workspace`

## 最小展示形态

Conversation 中渲染一条 compact event cell：

- 第一行：workflow icon、`Feature Development`、状态 badge、短 run id、更新时间。
- 第二行：当前 stage 和 typed summary，例如 `Review/Fix in progress - waiting for reviewer`。
- 第三块：stage rail/list，例如 `Research done -> Implement done -> Review/Fix running -> Verify pending`。

桌面优先横向 rail；窄屏切换纵向 stage list。该 cell 不是大卡片，不嵌套其他卡片，不提供直接操作按钮。

## Typed Payload 建议

如果现有 `ThreadItem` 类型缺少 workflow progress variant，建议补：

```text
ThreadItem::WorkflowRunProgress {
  id,
  runId,
  workflowId,
  workflowName,
  status,
  currentStageId,
  message,
  stages[],
  edges?,
  updatedAtMs
}
```

`stages[]` 建议字段：

```text
{
  id,
  label,
  status,
  agentLabel?,
  startedAtMs?,
  completedAtMs?,
  errorMessage?
}
```

状态枚举：

- run status：`queued`、`running`、`waiting`、`completed`、`failed`、`aborted`
- stage status：`pending`、`running`、`waiting`、`completed`、`failed`、`skipped`

## Thread / Agent 所属关系

Progress card 展示 run 进展；thread / agent badge 只展示当前 thread 属于哪个 workflow/run/stage。

Agent Tree：

- 在 agent 行第二行或右侧小 chip 显示 `Workflow · feature-dev · Review/Fix`。
- 保持原有 root/subagent 树结构和排序，不新增 workflow 分组，不画 workflow graph。
- chip 的 tooltip/aria-label 显示完整 `Feature Development · run wf_42a9 · Review/Fix`。
- 如果空间不足，显示 `WF · Review/Fix`，完整内容进 tooltip/aria-label。

Conversation Header：

- 在 thread path / run config 附近显示低权重 chip：`Feature Development · Review/Fix · run wf_42a9`。
- chip 可选点击定位到最近 workflow progress item；只有 typed item id 可用时才表现为按钮。
- header chip 不展示进度百分比，不替代 progress card。

后端只给 thread metadata workflow binding 时：

- 直接显示 badge/chip。
- 进度卡仍等待 `ThreadItem::WorkflowRunProgress`；不能用 metadata 合成 progress card。

后端没有 metadata 时：

- 不显示 thread/agent badge，避免误导。
- 如果 conversation 中有 progress card，也不能反推当前 thread 所属 workflow。
- 开发验收记录断点：`Progress is visible, but thread-to-workflow binding metadata is unavailable.`
- 如需要调试提示，只放在 Thread Analysis/debug 区低权重显示，不进主 Agent Tree。

## 文案

- queued：`Workflow queued`
- running：`Workflow running`
- waiting：`Workflow waiting`
- completed：`Workflow completed`
- failed：`Workflow failed`
- aborted：`Workflow aborted`
- 缺少图详情：`No graph details in this update.`
- typed fallback：`Workflow updated`、`Progress details unavailable`
- 无 binding metadata 调试提示：`No workflow binding metadata for this thread.`

失败原因必须来自 typed `errorMessage` / `failureReason`，不可从 raw output 或 assistant text 提取。

## 与 Slash Menu 的关系

Slash workflow 候选继续来自 app-server v2 `workflow/list` discovery。

选择候选时：

- 客户端只生成可编辑草稿。
- 用户发送草稿后，由模型在当前 turn 中调用 `workflow_start`。
- 进度出现的唯一 UI 来源是 typed workflow progress item。
- thread 所属 badge 的唯一来源是 thread metadata workflow binding。
- 如果 `workflow_start` 尚未发出 progress item，conversation 不伪造“已启动”卡片。

## 交互分支

- payload 有 stages：展示 stage rail/list。
- payload 无 stages：展示 header + summary + 缺图详情文案。
- 同一 item id 更新：更新同一 cell。
- 不同 item id：追加新 entry，即使 run id 相同。
- failed stage：stage 显示失败 badge，错误原因两行截断。
- completed run：stage 全部保留，terminal 状态显示 success。
- compact archive：按 typed item id 和 server 顺序保留，不能折叠为普通 text。

## 可访问性与响应式

- cell 整体 aria-label 包含 workflow name、run status、current stage、stage 数量。
- stage 状态必须有可见文字；颜色只作为辅助。
- stage rail 低于约 560px 改为纵向列表。
- stage label 单行截断，完整文本进入 tooltip/aria-label。
- Agent Tree badge 不应挤压 agent path；窄宽度优先截断 badge label，保留 agent 名称。
- Header chip 在窄宽度下折叠为 `Workflow · Stage`，run id 隐藏到 tooltip/aria-label。
- Header chip 只有在存在可定位的 typed workflow progress item id 时才使用 button/link 视觉；没有定位目标时必须是只读 chip，不能出现 hover cursor 或可点击样式。
- 如果 stage 可展开，使用 button 语义和 visible focus ring；最小版本可只读，减少 Tab 噪音。
- 搜索/定位只基于 `ConversationEntry` / `ConversationCell`，定位后高亮 cell，不改变 composer draft。

## 开发 Handoff

实现入口建议：

- protocol/types：新增或确认 `ThreadItem::WorkflowRunProgress` typed payload。
- thread metadata：新增或确认 workflow binding metadata，覆盖 workflow id/name、run id、stage id/label、binding role/status。
- projector：确保 `EventMsg::WorkflowRunProgressCompleted` 生成 typed ThreadItem。
- root-worker types：添加 workflow progress item 类型和 workflow binding metadata 类型，不接 raw fallback。
- conversation mapping：`buildConversationItemEntries` 将 workflow progress 映射为 event/workflow entry，保留 `ThreadItem.id`。
- rendering：新增 `WorkflowProgressCell` 或扩展 `EventRow` 支持 `eventCategory="workflow"`。
- Agent Tree/header：新增 `WorkflowThreadBadge`，只消费 metadata，不从 progress card 反推。
- virtualization：如 cell 高度超过普通 event row，需要更新高度估算或依赖稳定测量。
- tests：覆盖 live item、thread/read snapshot、same id update、different id append、missing stages fallback、failed stage、waiting stage、skipped stage、metadata badge、有 progress 无 metadata 断点、narrow layout class。
- visual verification：实现前或实现中补一次约 560px 以下窄屏截图，确认 stage label、badge、header chip 不重叠。

## 验收

- `/` workflow 候选选择后只写草稿，不直接启动 workflow。
- 模型启动 workflow 后，typed progress item 出现在 conversation。
- feature-dev 至少展示 `Research`、`Implement`、`Review/Fix`、`Verify` 四个 stage 的状态。
- workflow 创建的 thread 如果有 metadata binding，Agent Tree 或 header 可见 workflow/run/stage 所属关系。
- 无 metadata binding 时不显示所属 badge，也不从 progress card 反推。
- 无 stages payload 不展示 raw JSON。
- failed/completed/aborted 状态文案明确。
- root-worker 不新增 raw marker、assistant JSON 或 legacy envelope parser。

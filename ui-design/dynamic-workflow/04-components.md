# 组件拆分与开发 Handoff

## WorkflowRunCard

职责：conversation 主列表中的 workflow run 摘要卡片。

输入：

- `runId`
- `definitionId`
- `title`
- `runnerStatus`
- `staticGraphSummary`
- `runtimeGraphSummary`
- `currentStageId`
- `activeAgentPaths`
- `failureSummary`
- `resumeState`

状态展示：

- `active`：蓝色/默认运行语义，显示当前 stage。
- `waiting_agent`：等待语义，显示等待的 agent 短名。
- `waiting_user`：需要用户操作，显示 gate 或 approval 名称。
- `completed`：成功语义，显示完成计数。
- `failed`：错误语义，显示失败摘要。
- `aborted`：中止语义，显示终止来源。

行为：

- 点击 `Open workflow` 打开右侧新增 Workflow Graph panel。
- 点击 agent count 或 `Agents` 定位到左侧 Agent Tree / `list_agents` 视图。
- 卡片可展开 details，但默认不展示完整 graph。

## WorkflowGraphPanel

职责：右侧新增 Workflow Graph panel 的总容器。它与 Thread Analysis 是并列能力，不替换 Thread Analysis。

区域：

- `WorkflowStatusStrip`
- `StaticGraphLane`
- `RuntimeNodeLayer`
- `WorkflowNodeDetails`

行为：

- 默认选中当前 active/failed/waiting 节点。
- 支持按 stage 折叠 runtime 节点。
- 支持过滤 `All / Active / Failed / Waiting / Agents only`。
- 支持复制 runId 和打开 agentPath。
- 点击已运行 `agent/thread` runtime 节点时，触发 `openThreadFromWorkflowNode(node)`：先用 `agentPath/threadId` 定位左侧 Agent Tree，再打开对应 conversation。
- 如果当前左侧 Agent Tree 折叠了父节点，需要自动展开到目标节点，并短暂高亮目标行。

键盘与焦点：

- `Tab` 进入 Workflow Graph panel 后，焦点顺序为 status strip actions、stage summary、runtime nodes、details actions。
- `Enter` 在已绑定 runtime agent/thread 节点上执行默认动作：定位 Agent Tree 并打开 conversation。
- `Space` 打开或切换 node details，不改变 conversation。
- `Esc` 从 details 返回触发节点；如果 focus 在 graph 空白处，Esc 关闭 panel 或返回上一个右侧 panel，由现有 RightPanel 规则决定。
- 动态新增 runtime 节点时不抢焦点；如果新增节点是当前等待对象，只更新 status strip 和 live region。

## StaticGraphLane

职责：展示 `staticGraph` 高层骨架。

node kind：

- `stage`：普通阶段。
- `branch`：decision 节点，显示分支 label 和 observed branch。
- `loop`：循环阶段，显示 iteration count。
- `parallel`：并行组，显示 active/completed count。
- `join`：汇合节点，显示等待计数。

聚合状态优先级：

```text
failed > waiting_user > waiting_agent > active > interrupted > completed > idle > unknown
```

## RuntimeNode

职责：展示实际发生的 runtime 节点。

字段：

- `id`
- `kind`
- `parent`
- `agentPath`
- `threadId`
- `status`
- `createdAt`
- `updatedAt`

kind：

- `agent`：显示 role/type、agentPath 短名、thread link。
- `thread`：显示 thread title/id 和状态。
- `shell`：显示 command 摘要和 exit 状态。
- `gate`：显示 approval/waiting_user 状态。

agent 状态推断：

- agent/thread status 为 running 或有 active turn：`running`。
- 终态成功：`completed`。
- 终态错误：`errored`。
- interrupted/shutdown：`interrupted`。
- binding 存在但 thread 未加载：`unknown`。
- binding path 不存在或 list_agents 不返回：`missing`。

点击行为：

- `completed/running/interrupted/errored` 且有 `agentPath`：默认打开对应 conversation，并在 Agent Tree 中选中该 agent。
- `unknown` 且有 `agentPath`：打开 details，主操作为 `Show in Tree`；如果 Agent Tree 能定位则允许定位高亮。
- `missing`：不改变 Agent Tree，打开 details 显示缺失原因和复制 path。
- `shell/gate`：默认打开 details，不跳转 Agent Tree。

可访问性：

- runtime node 使用 button 语义，accessible name 格式：`<node label>, <kind>, <status>, <default action>`。
- 状态文本必须可见或在 aria label 中出现；颜色点只是辅助。
- 被选中节点使用 `aria-current` 或等效 selected 状态；不只依赖蓝色边框。
- 连接线和 branch/parallel/join glyph 不作为独立交互元素，除非它们有可点击行为；否则用 `aria-hidden`。

## LoopIterationGroup

职责：处理动态新增 reviewer/fix 节点。

规则：

- 从 runtime node id 中识别常见 suffix：`reviewer-0`、`fix-0`、`reviewer-1`。
- 如果 runtimeGraph 明确提供 iteration metadata，优先使用 metadata。
- 无 metadata 时按同一 loop stage 下的创建顺序分组，避免错误重排。
- 当前 iteration 展开；历史 iteration 默认折叠为 summary。

## AgentPathLink

职责：连接 workflow runtime node 与现有 subagent 体系。

展示：

- 列表显示短 path，例如 `reviewer-1`。
- hover/focus 或 details 显示完整 `agentPath`。
- 提供 `Open thread`、`Copy path`、`Show in Tree`。
- `Open thread` 与点击 runtime node 的默认行为一致：定位 Agent Tree 并打开对应 conversation。
- `Show in Tree` 只做定位和高亮，不切换 conversation。该按钮用于用户只想确认 subagent 在树中的位置。

约束：

- 不通过字符串解析 message 找 agentPath。
- 如果 `list_agents` 未返回该 path，显示 `Missing from agents list`，保留复制入口。

## WorkflowNodeDetails

职责：选中节点的完整调试信息。

内容：

- static node id/title/kind。
- runtime node id/kind/parent。
- runnerStatus 和 node status。
- agentPath、threadId、binding source。
- last event、error summary、resume marker。
- raw typed payload 的结构化摘要。

焦点与辅助技术：

- 打开 details 时，焦点进入 details 标题；第一个可操作按钮是 `Open thread`。
- details 中完整 `agentPath` 可复制，不能只放在 hover tooltip。
- 关闭 details 后，焦点回到触发的 runtime node。
- error details 使用可读文本摘要，不只展示红色状态。

## Prototype

可视化 prototype 资产：

- [baseline-thread-empty.png](/Users/bytedance/Projects/my-codex/ui-design/dynamic-workflow/assets/baseline-thread-empty.png)
- [workflow-panel-added-on-baseline.svg](/Users/bytedance/Projects/my-codex/ui-design/dynamic-workflow/assets/workflow-panel-added-on-baseline.svg)
- [workflow-panel-added-on-baseline.png](/Users/bytedance/Projects/my-codex/ui-design/dynamic-workflow/assets/workflow-panel-added-on-baseline.png)
- [workflow-right-panel-added.svg](/Users/bytedance/Projects/my-codex/ui-design/dynamic-workflow/assets/workflow-right-panel-added.svg)
- [workflow-right-panel-added.png](/Users/bytedance/Projects/my-codex/ui-design/dynamic-workflow/assets/workflow-right-panel-added.png)
- [workflow-graph-prototype.svg](/Users/bytedance/Projects/my-codex/ui-design/dynamic-workflow/assets/workflow-graph-prototype.svg)
- [workflow-graph-panel-exploration.png](/Users/bytedance/Projects/my-codex/ui-design/dynamic-workflow/assets/workflow-graph-panel-exploration.png)

`workflow-panel-added-on-baseline.svg/png` 是本轮主设计图：真实 baseline 原样保留，只在最右侧新增 Workflow Graph panel。`workflow-right-panel-added.*` 与 `workflow-graph-panel-exploration.png` 是早期探索稿，不作为最终结构约束。

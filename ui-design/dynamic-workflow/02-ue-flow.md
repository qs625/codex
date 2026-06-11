# UE Flow

## 主路径：启动并观察 workflow

1. 用户在 thread 中触发 `workflow_start`。
2. conversation 出现 Workflow Run 卡片，状态为 `active`，显示 workflow 名称、runId 短码、当前 stage 和已创建 agent 数。
3. 右侧 rail 新增 `Workflow` 入口；用户打开新增 Workflow Graph panel。现有 Thread Analysis panel 和左侧 Agent Tree 保持原样。
4. Workflow Graph panel 先展示 `staticGraph` skeleton，未发生的 stage 为 idle。
5. runner 通过 `runtimeGraph` 更新绑定真实节点，runtime 节点挂到对应 static stage 下。
6. 用户看到某个 stage 下新增 agent 节点，例如 `owner`，节点展示 `agentPath` 短名和派生 status。
7. 用户点击已运行的 agent/thread runtime 节点，UI 自动展开并高亮左侧 Agent Tree 中的对应节点，同时中间 conversation 切换到该 subagent thread；也可通过 `list_agents` 入口查看完整 subagent 树。

## 图节点到 Agent Tree / Conversation 跳转

1. 用户在 Workflow Graph panel 点击 `reviewer-1` runtime 节点。
2. 客户端读取该节点的 `agentPath` 或 `threadId` binding。
3. 左侧 Agent Tree 自动展开到目标 path，目标行短暂高亮。
4. 中间 conversation 打开目标 agent thread。
5. 右侧 Workflow Graph 保持当前选中节点，并打开 `Node details`，方便用户确认 status source、binding 和 path。

失败分支：

- 找不到 `agentPath`：Workflow Graph 只打开 details，显示 `binding missing`，不改变 Agent Tree selection。
- `agentPath` 存在但 Agent Tree 尚未加载：显示 `thread not loaded`，保留 `Retry locate` / `Copy path`。
- 目标 agent 已终止或 errored：仍允许跳转 conversation，节点状态用现有 agent/thread status 标识。

## 等待 agent

1. runnerStatus 变为 `waiting_agent`。
2. Workflow Run 卡片显示 `Waiting for agent`，并列出最多 2 个 active/waiting agent 短名。
3. Graph 中对应 runtime agent 节点为 running 或 waiting 派生状态，static stage 聚合为 in-progress。
4. 如果多个 agent 并行运行，parallel group 展示 active count。

## branch

1. staticGraph 中 branch 节点显示为 decision row，列出分支 label。
2. 未选择分支保持 muted。
3. runtimeGraph 出现实际分支节点时，该分支高亮为 observed。
4. 未发生分支不显示失败，只显示 `not observed`。

## loop

1. staticGraph 中 loop stage 显示循环 badge，例如 `Review/Fix loop`。
2. runtimeGraph 新增 `reviewer-0`、`fix-0`、`reviewer-1` 时，Graph 按 iteration 分组：

```text
Review/Fix
  Iteration 0
    reviewer-0
    fix-0
  Iteration 1
    reviewer-1
```

3. 当前 iteration 展开，历史 iteration 可折叠。
4. loop stage 聚合状态取当前 iteration 中最高优先级状态：failed > waiting_user > running > interrupted > completed > idle。

## parallel / join

1. parallel stage 显示为 lane group，子节点纵向排列。
2. 每个 parallel runtime 节点独立显示 agent/thread 状态。
3. join 节点显示等待计数，例如 `Join 2/3 completed`。
4. 全部 parallel 子节点完成后，join 变为 completed，后续 stage 激活。

## 失败

1. runnerStatus 为 `failed` 时，Workflow Run 卡片显示失败摘要和失败 stage。
2. Graph 中 runner failure 显示在顶部 status strip；如果能关联 runtime node，该节点同步标红。
3. details 展示 error message、runner stderr/log 摘要、last event id、runId。
4. 可见操作：`Retry/resume` 入口仅在后端支持时出现；第一版设计为 disabled/hidden，由实现能力决定。

## 完成

1. runnerStatus 为 `completed`。
2. Workflow Run 卡片显示 completed、总 agent 数、完成 stage 数。
3. Graph 所有 observed runtime 节点保持可访问，未发生分支仍为 muted `not observed`。
4. details 保留 final summary 和 bindings。

## resume

1. 用户触发 `workflow_resume`。
2. Workflow Run 卡片出现 `Resumed` 次级标记，runnerStatus 从 `active` 或 `waiting_agent` 继续推进。
3. 已存在 binding 的 agent 节点显示 `rebound` indicator，不重新创建 runtime 节点。
4. 如果 binding 存在但 thread/agent status 暂不可用，节点显示 `unknown`，details 中展示 `agentPath` 和原因。
5. 如果 binding 缺失，stage 显示 `binding missing`，runnerStatus 多数情况下为 `failed` 或 `waiting_user`，由 app-server 状态决定。

## 空状态与加载

- 没有 workflow run：Graph 面板显示空态 `No workflow run in this thread`，附带最近可用入口说明，不在 conversation 主列表插入假卡片。
- 已收到 run 但 graph 未加载：Workflow Run 卡片显示 skeleton metadata；Graph 显示 stage skeleton。
- thread status 未加载：runtime node 保留 `unknown` 状态点，避免误判为 idle 或 failed。

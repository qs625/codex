# 信息架构

## 页面结构

```text
Root Worker Prototype
├─ Left / Agent Tree
│  └─ 继续展示现有 agent/thread 父子层级
├─ Center / Conversation
│  ├─ User / assistant / tool items
│  └─ WorkflowRunCard
└─ Right Workspace
   ├─ Rail
   │  ├─ Analysis
   │  ├─ Workflow
   │  └─ Agents
   ├─ ThreadAnalysisPanel
   │  └─ 现有 context / monitors / events / skills 分析
   └─ WorkflowGraphPanel
      ├─ Status strip
      ├─ Static graph skeleton
      ├─ Runtime nodes by stage
      └─ Node details drawer
```

## Conversation 中的 Workflow Run 卡片

用途：让用户不打开 Graph 也能知道 workflow 当前状态。

默认信息：

- title：workflow title 或 definition id。
- status pill：runnerStatus。
- progress：observed stages / total static stages。
- active focus：当前 stage、等待 agent、失败节点。
- quick actions：open graph、copy run id、view agents。

默认高度控制在 3 到 5 行。长 error 和完整 bindings 不在主卡片展开。

## 右侧新增 Workflow Graph panel

用途：作为独立 panel 展示完整 workflow 图和 runtime 绑定。它不替换 Thread Analysis；在右侧新增一个 Workflow Graph panel。主设计图见 [workflow-panel-added-on-baseline.png](/Users/bytedance/Projects/my-codex/ui-design/dynamic-workflow/assets/workflow-panel-added-on-baseline.png)。

层级：

- Status strip：runnerStatus、runId、resume marker、started/updated 时间。
- Static graph lane：高层流程，表达 stage/branch/loop/parallel/join。
- Runtime node rows：挂在 static stage 下，表达真实 agent/thread/shell/gate。
- Details drawer：选中节点后的完整 metadata。

与现有区域关系：

- 左侧 Agent Tree：保持现有层级、折叠、状态点和 conversation 导航职责。
- Workflow Graph：只负责解释 workflow stage 与 runtime node 的对应关系。
- Thread Analysis：继续负责 context、monitors、events、skills 等 thread 诊断。

节点点击行为：

- 点击已绑定的 `agent/thread` runtime node：同步选中左侧 Agent Tree 中的对应 agent，并打开该 agent 的 conversation。这是主点击、Enter 和 `Open thread` 的统一行为。
- 点击未绑定节点：打开 details，显示 `binding missing` 或 `thread not loaded`。
- 点击 stage 空白区域：只选中 stage，不改变左侧 Agent Tree 当前 selection。
- `Show in Tree` 是辅助动作，只定位并高亮左侧 Agent Tree，不切换 conversation。

## Runtime 节点挂载规则

- `runtimeNode.parent` 指向 staticGraph node id 时，直接挂到对应 stage。
- parent 指向 runtime node 时，作为 runtime 子节点缩进挂载。
- parent 缺失或 static node 不存在时，放入 `Unmapped runtime nodes` section，并显示 warning。
- agent 节点显示 `agentPath` 短名；完整 path 在 details。
- 已绑定 `agentPath` 的 runtime 节点必须提供两个入口：`Open thread` 同时定位 Agent Tree 并打开 conversation；`Show in Tree` 只定位和高亮树节点。

## 响应式策略

宽屏，窗口宽度 >= 1440px：

- 右侧可同时容纳 Thread Analysis 与新增 Workflow Graph panel。
- Thread Analysis 最小宽度 280px，Workflow Graph 最小宽度 360px；如果宽度不足，Thread Analysis 保持原 panel，Workflow Graph 可通过横向滚动或 rail 打开。
- Workflow Graph 使用横向 stage lane，runtime 节点在每个 stage 下纵向排列。
- Details drawer 可作为 Workflow Graph 内右侧内嵌栏。

中等宽度，窗口宽度 1024px 到 1439px：

- 右侧一次只显示一个 panel；`Workflow`、`Analysis`、`Agents` 通过 rail 切换。
- Graph 保持横向滚动，stage 宽度固定在 120px 到 160px，runtime 节点不压缩到不可读。
- Status strip 固定在顶部。
- 长 `agentPath` 只显示短名和 middle truncation，完整 path 放 details。

窄屏，窗口宽度 < 1024px：

- Graph 改为纵向 timeline。
- Details drawer 变为底部 overlay 或单独 panel。
- Conversation 卡片只保留 status、title、current focus，一行 action icon。
- Thread Analysis 与 Workflow Graph 都通过 rail/overlay 打开，不与 conversation 并列挤压。
- 节点密集时，loop iteration 和 parallel group 默认折叠为 summary。
- 单个 runtime 节点默认高度不超过 44px，长 label 使用一行截断；详情中保留全文。

## 可访问性

- runnerStatus 和 node status 不能只靠颜色表达，必须有文本 label 或 aria label。
- Graph 中节点可键盘聚焦，Enter 打开 details，Esc 返回 graph。
- 绑定到 thread 的节点支持 Enter 执行默认动作：打开对应 conversation；`Space` 或二级按钮打开 details。
- stage lane 的阅读顺序与流程顺序一致。
- branch/parallel/loop 的 glyph 必须配合文本，如 `Branch`、`Loop`、`Parallel`。
- 动态新增节点需要使用 polite live region 或 conversation item 更新，不打断用户当前焦点。
- runtime 节点使用 button 语义，accessible name 包含 stage、node id、kind、status 和默认动作，例如 `reviewer-1, agent, running, opens thread`。
- 打开 details 后焦点进入 details 标题；关闭 details 后焦点回到触发节点。

# Dynamic Workflow UI Brief

## 产品目标

Dynamic Workflow 需要在 root-worker prototype 中把“可脚本化、可恢复的工程流程”变成可扫描的 UI。用户应能在 thread 页面看到 workflow run 的当前状态，在右侧新增的 Workflow Graph panel 中理解预期流程、实际执行节点、subagent 关系和 resume 后的绑定状态。

核心体验不是流程编辑器，而是运行观察器：

- `staticGraph` 展示预期的高层流程骨架。
- `runtimeGraph` 展示已经发生的 agent/thread/shell/gate 节点。
- 真实 agent 节点状态从现有 thread/agent status 推断，不在 workflow UI 中另存一套状态。
- runner 状态说明 workflow runner 自身是在执行、等待 agent、等待用户、失败、完成或已终止。

## 目标用户

- 角色：使用 my-codex 多 agent 协作、workflow 脚本和 root-worker prototype 的工程师、PM agent 调试者、平台开发者。
- 使用频率：高频查看运行中 workflow，偶尔展开失败节点和 resume 绑定详情。
- 设备：桌面端为主，支持窄宽度 Electron/web renderer。
- 专业程度：熟悉 thread、agent path、subagent、list_agents、followup、resume 等概念。

## 范围

涉及：

- thread 页面 conversation 中的 workflow run 卡片。
- 右侧新增 Workflow Graph panel 中的 staticGraph + runtimeGraph 可视化。
- 保留现有 Thread Analysis panel 和左侧 Agent Tree，不用 workflow 图替换已有 agent 层级视图。
- runnerStatus、agent/thread 派生状态、bindings、动态新增节点的展示规则。
- workflow 创建的 subagent 关系、`agentPath`、`list_agents` 入口和跳转策略：流程图中的已运行 thread/agent 节点点击后，应定位到左侧 Agent Tree 对应节点，并打开对应 conversation。
- 空状态、运行中、等待、失败、完成、aborted、resume 后状态。

不涉及：

- 实现前端代码、Rust/TS runtime 代码或 app-server 协议。
- 完整 BPMN 编辑器、流程图拖拽编辑、自动从 TypeScript AST 推断流程。
- 新增 workflow DSL 设计。
- 改造现有 agent execution 语义。

## 约束

- 新 workflow 展示必须走 typed `ResponseItem -> ThreadItem -> client UI`，不能靠 raw message 或文本解析。
- conversation 主列表只放短摘要和关键状态；完整 graph、bindings、debug metadata 进入右侧 Graph 或 details。
- 第一版只能表达 `staticGraph` 声明的骨架和 `runtimeGraph` 已发生节点，不承诺展示普通 TypeScript 所有真实分支。
- runner 状态来自 app-server workflow run 状态，客户端不探测 Node runner 进程。
- agent 节点状态使用现有 thread/agent 派生状态：`running`、`completed`、`errored`、`interrupted`，以及 thread 未加载时的 `unknown`。
- 文案以英文 UI token 为主，设计文档使用中文说明。
- 工程工具视觉方向：信息密度高、低装饰、强层级、可键盘访问。

## Baseline 记录

本任务涉及现有 root-worker prototype 客户端 UI，已按固定环境尝试获取 baseline：

- `CODEX_HOME=/tmp/my-codex-root-worker-ui-env/codex-home`
- `ROOT_WORKER_WORKSPACE=/tmp/my-codex-root-worker-ui-env/workspace`

尝试路径：

- 使用 `$playwright-cli` 打开 `http://127.0.0.1:5173` 并截图，Playwright CLI 在打开 browser session 时未返回截图。
- 使用 Computer Use 查询窗口，工具调用超时。
- 使用系统截图 `screencapture`，命令未即时返回，疑似 macOS 屏幕录制权限或当前会话窗口状态限制。

最终通过 macOS `screencapture` 成功获取 baseline，文件：

- [baseline-thread-empty.png](/Users/bytedance/Projects/my-codex/ui-design/dynamic-workflow/assets/baseline-thread-empty.png)

该截图显示当前界面由左侧 Agent Tree、中间 conversation、右侧 Thread Analysis panel 和最右侧竖向工具 rail 组成。本设计基于该结构新增 Workflow Graph panel，保留现有 Agent Tree 和 Thread Analysis。

## 验收标准

- thread 页面中单个 workflow run 卡片默认高度稳定，能显示 run id/title、runnerStatus、当前 stage、active agent 数、失败摘要。
- 右侧 Workflow Graph 是新增 panel/rail 入口，不替换 Thread Analysis；Thread Analysis 仍可独立访问。
- 左侧 Agent Tree 继续作为 agent/thread 层级的权威导航，不被 workflow 图重画或取代。
- staticGraph 能明确表达 `stage`、`branch`、`loop`、`parallel`、`join` 的高层语义。
- runtimeGraph 节点能挂在 static stage 下，并展示实际 `agentPath`、thread 入口和派生状态。
- 已运行 runtime thread/agent 节点点击后，应定位到 Agent Tree 对应节点，并打开对应 conversation。
- runnerStatus 六类状态均有明确视觉语义：`active`、`waiting_agent`、`waiting_user`、`completed`、`failed`、`aborted`。
- 动态新增 `reviewer-0`、`fix-0`、`reviewer-1` 等节点不会造成布局跳动失控，能按 loop iteration 分组。
- resume 后能区分“已恢复并复用 binding”和“绑定缺失/待重新连接”。
- 空状态、加载中、失败、完成和无 workflow 的 thread 都有可读状态。
- `list_agents`/agentPath 入口清晰，可从 workflow 节点进入对应 agent/thread。

# 调研与模式参考

## Spec 摘要

`spec/dynamic-workflow.md` 明确 Dynamic Workflow 的展示模型是两层图：

- `staticGraph`：definition 中声明的高层流程骨架，节点 kind 包括 `stage`、`branch`、`loop`、`parallel`、`join`。
- `runtimeGraph`：执行时逐步填充真实节点，包括 `agent`、`thread`、`shell`、`gate` 等。

UI 需要把 workflow agent id 与已有 agent session/thread path 的 binding 展示出来。`Agent(id)` 是幂等的：首次运行创建 subagent，resume 时绑定回已有 session，不重复 spawn。

runner 状态由 app-server workflow run 提供，客户端不直接检测 Node runner 进程。agent 节点状态从既有 thread/agent status 推断。

## 现有 root-worker 模式

可复用模式：

- conversation 列表已经有 typed item 投影：tool call、event command、agent status 等都适合以“短标题 + metadata + 可展开 details”展示。
- RightPanel 已有 Analysis/Graph rail，Graph 入口可承载 workflow 图，不需要把完整图塞进主列表。
- AgentTree 已有 thread/agent 父子关系、状态点、折叠计数和层级缩进，适合 runtimeGraph 的 agent 子树。
- Thread Analysis 已有 summary metric + sections 的信息组织，适合 workflow summary、active gates、blocked agents。

设计原则：

- 主列表回答“这个 workflow run 现在怎么了”。
- 右侧 Graph 回答“预期流程是什么，实际跑到了哪里”。
- Details 回答“这个节点绑定到哪个 agentPath/threadId，有哪些 raw/debug metadata”。

## 同类设计模式

工程工具中的 workflow/CI/trace UI 通常采用三层结构：

- run summary card：状态、耗时、失败点、当前等待对象。
- graph/timeline：阶段顺序、分支、并行、循环迭代。
- node details：日志、资源 id、重试、跳转链接。

Dynamic Workflow 更接近 CI pipeline + distributed trace 的组合：

- `staticGraph` 类似 pipeline definition。
- `runtimeGraph` 类似 trace spans。
- agent/thread status 类似外部 span 的真实运行状态。

因此不建议第一版做自由画布。更稳妥的布局是“左到右 stage lane + 每个 stage 下的 runtime rows”，可读性强，也更容易支持窄屏。

## 风险

- 静态图不等于真实执行路径。UI 必须显式标注 runtime node 是“observed”，避免用户以为没有出现的分支就是不会执行。
- loop 会不断新增节点，必须按 iteration 或 node id 序号分组，避免无限横向扩张。
- parallel 和 join 容易在小屏上拥挤，第一版应优先使用纵向 lane，而不是复杂曲线。
- `agentPath` 可能很长，列表中只展示短名，完整 path 放 details 和复制入口。
- resume 后可能出现 binding 存在但 thread 未加载、thread 已删除、agent status 暂不可用的状态，需要 `unknown`/`missing` 视觉语义。

## 设计决策

- workflow run card 放在 conversation 主流中，作为 typed item 的摘要卡片。
- 右侧 Graph 面板成为主要可视化入口，默认按 static stage 分列，每列下挂 runtime 节点。
- branch/loop/parallel/join 用轻量 glyph + 文本 label 表达，不引入复杂 BPMN 造型。
- runtime 节点使用状态点、role label、agentPath 短名和 thread 跳转。
- 动态新增节点采用 append-only 分组，已有节点位置不重排。

# Dynamic Workflow 设计方案

## 背景

当前 agent 协作主要依赖单轮 ReAct、人工 PM 编排和多 agent primitive。对于工程任务，外层流程通常是稳定的，例如调研、实现、review、验证、合并；真正需要探索的是每个阶段内部。

Dynamic Workflow 的目标是把外层流程变成可脚本化、可展示、可恢复的编排能力，同时保留每个 agent session 内部的 ReAct 自主执行。

## 目标

- 通过 TypeScript workflow 脚本固定 agent session 的执行流程。
- 让 agent session 成为 workflow 中的长期对象，而不是一次性函数调用。
- 支持脚本中使用普通 TypeScript 表达分支、循环、并行和反馈。
- 通过静态流程图展示预期流程，通过运行时图填充实际 agent/thread 节点。
- 保留 workflow 创建的 subagent 与父 thread 的关系，兼容现有 `list_agents`、`followup_task`、child completion 语义。
- 支持 resume：重新执行 workflow 脚本，并让 `Agent(id)` 绑定回已有 agent session。
- 用 `EventMsg::WorkflowRunProgressCompleted -> ThreadItem::WorkflowRunProgress` lifecycle 展示 workflow 进展；`ResponseItem::WorkflowRunProgress -> EventMsg::ResponseItemCompleted` 只保留旧 rollout/history 兼容和模型历史双写，避免 raw message 和文本解析作为新链路。

## 非目标

- 第一版不实现完整 BPMN 引擎。
- 第一版不做从 TypeScript AST 自动推断完整流程图。
- 第一版不做复杂可视化 workflow 编辑器。
- 第一版不强制所有分支、循环、并行通过 DSL primitive 表达。
- 第一版不在 Rust 进程内嵌 JavaScript 引擎。

## 核心模型

### Workflow Definition

Workflow 脚本是 TypeScript module，默认导出一个 definition object：

```ts
export default defineWorkflow({
  id: "feature-dev",
  version: "1.0.0",
  staticGraph: {
    nodes: [
      { id: "research", title: "Research", kind: "stage" },
      { id: "implement", title: "Implement", kind: "stage" },
      { id: "review_fix", title: "Review/Fix", kind: "loop" },
      { id: "verify", title: "Verify", kind: "stage" }
    ],
    edges: [
      ["research", "implement"],
      ["implement", "review_fix"],
      ["review_fix", "verify"]
    ]
  },
  async run(wf) {
    // workflow logic
  }
});
```

Node 直接执行的不是用户脚本，而是 Codex 提供的 runner。runner 加载 workflow definition，创建 `wf` runtime object，并调用 `definition.run(wf)`。

### Agent Session Handle

Workflow 中的 agent 是长期 session handle：

```ts
const owner = await wf.Agent("owner", {
  parent: "implement",
  type: "refactor-owner",
  cwd,
  message: initialPrompt
});

await owner.wait();
await owner.followup("修复 review findings");
await owner.wait();
```

`Agent(id)` 是幂等的：

- 首次运行时创建 subagent，并记录 `workflowRunId + agentId -> agentPath`。
- resume 时返回已有 agent session handle，不重复 spawn。
- agent 仍然是普通 subagent，保留父子 thread 关系。

重复 followup 不由 workflow runtime 做严格去重。agent session 自身应能根据上下文判断任务已完成，避免重复执行无意义动作。对于 shell、发布、删除、发邮件等非 agent 副作用，仍需要显式 durable step 或 approval。

### Workflow Graph

Workflow 图分两层：

- `staticGraph`：definition 中声明的高层流程骨架，供客户端预先展示。
- `runtimeGraph`：执行时逐步填充的真实节点，包含实际 agent/thread、shell、gate 等节点。

`staticGraph` 可以使用 BPMN-like 最小子集表达高层语义：

- `stage`
- `branch`
- `loop`
- `parallel`
- `join`

不引入完整 BPMN 语法和执行引擎，只复用成熟流程图概念。

`runtimeGraph` 节点示例：

```json
{
  "id": "owner",
  "parent": "implement",
  "kind": "agent",
  "agentPath": "/root/project_pm/wf_123_owner"
}
```

客户端根据 `staticGraph` 画预期流程，根据 `runtimeGraph + thread/agent status` 推断实际进度。

## 执行架构

### 进程模型

Rust host 创建 durable `WorkflowRun`，然后启动 Node runner 子进程：

```text
Rust host
  -> node workflow-runner.mjs --mode run --bundle workflow.bundle.mjs --run-id wf_123
```

runner 与 Rust host 通过父子进程 stdio 管道进行 NDJSON RPC：

```text
Node stdout -> Rust host 读取 RPC request
Node stdin  <- Rust host 写入 RPC response
Node stderr -> 普通日志
```

workflow 脚本不能直接调用真实系统能力。`wf.Agent`、`wf.shell`、`wf.emit` 等方法都通过 RPC 回调 Rust host；只有 `workflow_start` / `workflow_resume` model tool 启动的 runner 会绑定当前 tool turn 的 runtime bridge，并由该 bridge 执行已有 agent、tool、shell、permission 逻辑。app-server v2 `workflow/*` 客户端控制面不绑定这层 bridge。

### TypeScript 支持

推荐流程：

1. `workflow_start` 找到 `.ts` entry。
2. snapshot workflow 目录、manifest 和 entry 相对路径。
3. runner 在 snapshot 目录中 import entry，并执行 `definition.run(wf)`。
4. resume 永远执行该 run 的 snapshot 目录，而不是仓库里的最新脚本。

这样可以避免 workflow 文件更新后破坏已有 run 的 resume 语义。

## Resume 语义

Workflow resume 不恢复 Node 调用栈，而是重新执行 workflow 脚本。

关键规则：

- `Agent(id)` 返回已有 agent session handle。
- `agent.wait()` 根据已有 agent status 返回结果或继续等待。
- `staticGraph` 与 snapshot bundle 保证同一 run 的流程定义稳定。
- runtimeGraph 记录 workflow node 与 agentPath 的绑定。
- workflow runner 只是执行器，状态 authority 在 app-server。

这与当前 thread resume 的思路一致：恢复的是持久 thread/agent 状态，而不是恢复运行时栈。

## 客户端状态

客户端不直接检测 Node runner 进程。客户端读取 app-server 的 workflow run 状态：

```json
{
  "runId": "wf_123",
  "status": "suspended",
  "runnerStatus": "waiting_agent",
  "staticGraph": {},
  "runtimeGraph": {},
  "bindings": {
    "owner": {
      "kind": "agent",
      "agentPath": "/root/project_pm/wf_123_owner"
    }
  }
}
```

agent 节点状态从已有 thread/agent 状态推断：

- `running`
- `completed`
- `errored`
- `interrupted`

workflow state 只保存图、绑定关系、runner 状态和非 agent 节点状态，不重复保存完整 agent 状态。

runner 可以处于：

- `active`：正在执行脚本。
- `waiting_agent`：挂起等待 agent。
- `waiting_user`：等待人工确认。
- `completed`：workflow 完成。
- `failed`：脚本或 runner 异常。
- `aborted`：用户终止。

## Tool/API 形态

Workflow 能力通过 agent session 可用的 tools 暴露：

- `workflow_start`
- `workflow_status`
- `workflow_resume`
- `workflow_abort`

当前实现了 durable runner 控制面：

- start：校验 registry 中的 workflow，创建 durable `WorkflowRun`，snapshot workflow 目录，并启动 Node runner。
- status：优先返回内存 run 状态，进程或会话恢复后可从 `$CODEX_HOME/workflow-runs/<runId>/run.json` 读取状态。
- resume：对未 abort 的 run 更新 inputs/revision，并基于 snapshot entry 重新执行 runner。
- abort：向 live runner 发送 abort 信号，停止子进程并持久化 aborted 状态；如果没有 live runner，也会把 durable run 标记为 aborted。

当前 runner 已能通过 Node 加载 workflow `.ts` module，并提供最小 `@codex/workflow` shim 支持 `defineWorkflow()`。workflow 目录内的相对 import 会随 snapshot 一起保留；仍不提供独立 TypeScript bundler/transpiler，脚本必须是当前 Node 运行时可加载的 `.ts`。通过 `workflow_start` / `workflow_resume` model tool 启动的 runner 会绑定当前 turn 的 runtime bridge：`wf.Agent` 通过 stdio NDJSON RPC 调用 host spawn/bind agent session，`agent.followup()` 和 `agent.wait()` 复用 MultiAgent V2 的 typed followup/wait 语义；同一 run 内的 agent binding 会持久化到 run snapshot，resume 时同 id 返回已有 binding。app-server v2 `workflow/*` 控制面仍只是客户端控制面，不绑定 runner-runtime bridge。`wf.shell` 第一阶段返回明确 unsupported RPC error，不继续静默占位，也不绕过 exec permission、hook 或 typed command lifecycle。

后续可以增加：

- `workflow_approve`
- `workflow_retry`
- `workflow_list`

## 展示模型

新增 workflow 相关展示语义时，应优先扩展 dedicated typed `EventMsg`，再统一投影到 `ThreadItem`。只有模型上下文或 provider history 需要感知时，才双写 typed `ResponseItem`。当前已实现的最小展示项是：

- `EventMsg::WorkflowRunProgressCompleted`
- `ThreadItem::WorkflowRunProgress`
- `WorkflowRunProgressKind::{Started, Resumed, Completed, Failed, Aborted}`

workflow tool 的 start/resume/abort 会写入模型历史所需的 typed `ResponseItem::WorkflowRunProgress`，并通过 dedicated `WorkflowRunProgressCompleted` lifecycle 投影为 `ThreadItem::WorkflowRunProgress`。客户端 live 展示不得从 function output JSON、assistant 文本或 marker 反解 workflow progress。
runner 进入 completed/failed 终态时会通过同一 typed lifecycle 追加终态 progress；显式 abort 由 `workflow_abort` 路径负责记录 Aborted progress，避免 start/resume 的旧 turn 重复记录 abort。app-server v2 直连控制面通过 `workflow/run/updated` notification 广播 run 状态更新，abort notification 也只由 `workflow/abort` 路径发送。

后续完整 runner 可能继续扩展：

- workflow graph updated
- workflow agent bound
- workflow runner suspended/completed/failed

这些不能通过 raw response item 或 message 文本解析作为新链路。

这与当前 item/message 架构收敛方向保持一致：

```text
typed EventMsg -> ThreadItem projector -> client UI
```

## 设计取舍

### 为什么不用完整 DSL

branch、loop、parallel 使用普通 TypeScript 表达。workflow runtime 不强制使用 `wf.loop`、`wf.branch` 这类 DSL primitive。

这样可以避免把 TS 变成另一套复杂流程语言。需要可视化时，由 `staticGraph` 给出高层骨架，由 `runtimeGraph` 填充真实执行。

### 为什么还需要 staticGraph

动态图无法预先知道完整执行路径。`staticGraph` 用于提前展示预期流程，例如：

```text
Research -> Implement -> Review/Fix -> Verify
```

实际执行时再填充：

```text
research.explorer
implement.owner
review_fix.reviewer
review_fix.fix_0
review_fix.reviewer
verify.tester
```

### 为什么不完全靠 thread 状态

thread 状态能表示 agent 是否 running/completed，但不能表达：

- agent 属于哪个 workflow run。
- agent 属于哪个 workflow stage。
- 动态节点挂在哪个 static stage 下。
- 非 agent 节点的状态。
- runner 是否正在协调或等待。

因此仍需要薄 workflow state 维护 graph binding。

## 开放问题

- `wf.shell` 后续是否接入现有 command/session 工具并提供 durable resume；接入前必须继续返回明确 unsupported error。
- workflow run state 当前存储在独立 `$CODEX_HOME/workflow-runs`，后续是否迁移到 thread-store/state DB。
- workflow typed `ResponseItem` 的粒度：单一 `WorkflowEvent` 还是多个具体 variant。
- 客户端是否使用 React Flow + ELK.js 实现 DAG 展示。
- workflow snapshot 目前是 workflow 目录 + shim，后续是否引入 bundle、transpile 和依赖锁定。
- workflow run 与当前 thread 的权限、cwd、approval policy 如何继承。

## MVP 范围建议

第一版已实现：

- TS workflow definition + runner。
- `workflow_start/status/resume/abort`。
- workflow run state 持久化。
- typed workflow response item 到 thread item 的展示。
- app-server v2 `workflow/list|describe|start|status|resume|abort` 控制面和 `workflow/run/updated` notification。

仍暂不实现：

- 完整 BPMN engine。
- 图形化 workflow editor。
- 复杂 durable shell step。
- AST 静态分析。
- 任意远程 workflow 脚本执行。
- `Agent(id)` 真实绑定/恢复 subagent session。
- 客户端展示高层流程图和实际 agent 节点状态。

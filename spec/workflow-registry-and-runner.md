# Dynamic Workflow Registry 与 Runner 设计

## 目标

本文补充 `dynamic-workflow.md` 中可并行推进的实现边界，聚焦 workflow registry / runner 本身，不依赖展示源迁移重构：

- project/home 两级 workflow registry。
- session init context 中的 workflow 摘要注入。
- TypeScript workflow runner 的执行模型。
- workflow 示例目录结构。

## Workflow 发现路径

Workflow 与 skills 类似，支持两级来源：

```text
$CODEX_HOME/workflows/
.codex/workflows/
```

推荐目录结构：

```text
.codex/workflows/<workflow-id>/
  WORKFLOW.md
  workflow.ts
```

单文件 workflow 可以作为后续便捷入口，但第一版推荐只支持目录形式，减少 metadata 和 entry 解析歧义。

## Registry 规则

扫描顺序：

1. `$CODEX_HOME/workflows/*/WORKFLOW.md`
2. `.codex/workflows/*/WORKFLOW.md`

冲突规则：

- project workflow 覆盖 home workflow。
- 同一来源出现重复 `id` 时，标记为 invalid，不注入 init context。
- `WORKFLOW.md` frontmatter 的 `id` 必须与目录名一致。
- `WORKFLOW.md` frontmatter 的 `entry` 必须指向同目录下的 TypeScript 文件。

`WORKFLOW.md` 是 workflow 的 canonical manifest 与说明文件。frontmatter 承载注册、展示和执行入口 metadata，正文承载面向模型和用户的 workflow 说明：

```markdown
---
id: feature-dev
name: Feature Development
description: 按调研、实现、review/fix、验证流程开发功能
entry: workflow.ts
version: "0.1.0"
inputs: {}
when_to_use: []
---
# Feature Development

按需描述使用场景、输入、执行阶段和当前能力边界。
```

Registry item 最小结构：

```json
{
  "id": "feature-dev",
  "source": "project",
  "path": ".codex/workflows/feature-dev",
  "entry": "workflow.ts",
  "name": "Feature Development",
  "description": "按调研、实现、review/fix、验证流程开发功能",
  "version": "0.1.0",
  "inputs": {},
  "when_to_use": []
}
```

## Init Context

Session 初始化时注入 workflow 摘要和 `WORKFLOW.md` 正文说明的截断内容，不注入完整 `workflow.ts`。

建议格式：

```text
## Available Workflows

Workflows are scripted, resumable multi-agent procedures. Use `workflow_list`
or `workflow_describe` when the user asks for a structured workflow and the task
matches one of the entries below. Use `workflow_start`, `workflow_status`,
`workflow_resume`, and `workflow_abort` to manage a workflow run.

- feature-dev (project)
  Name: Feature Development
  Description: 按调研、实现、review/fix、验证流程开发功能。
  Instructions:
    按需描述使用场景、输入、执行阶段和当前能力边界。
  Use when: 新功能、复杂 bugfix、需要 owner/reviewer 流程。
  Inputs: objective, cwd
  Inspect: workflow_describe({"workflow": "feature-dev"})
```

注入规则：

- 每个 workflow 的正文说明应按 context 预算截断。
- 不注入 TypeScript 源码或完整 staticGraph。
- 详细信息通过 `workflow_describe` 或 `workflow_status` 按需读取。

## 三层控制面边界

Dynamic Workflow 需要明确区分三层入口，避免把 client RPC、模型 tool 和 runner runtime bridge 混在一起：

1. Model tool：`workflow_list/describe/start/status/resume/abort`。这是当前主路径，模型根据 init context 判断是否需要 workflow，并自行调用 `workflow_start` 等工具。
2. Client/app-server RPC：app-server v2 的 `workflow/list|describe|start|status|resume|abort`。它是客户端控制面，不是当前重点，也不作为 TypeScript runner 调用 runtime 能力的桥。该控制面没有当前 model turn 的 `Session`/`TurnContext`，不得伪造 thread/session context 来执行 agent 或 shell 能力。
3. Runner-runtime bridge：TypeScript SDK 中的 `wf.Agent`、`agent.followup()`、`agent.wait()`、`wf.shell()` 到 Codex runtime 的真实能力桥。这一层只能在 `workflow_start`/`workflow_resume` tool 启动 runner 时绑定当前 turn/runtime context。

已决策：

- 不优先实现 slash command 直接启动 workflow。
- 当前主路径是模型读取 init context 后自行调用 workflow tool。
- app-server `workflow/*` RPC 保持为控制面，不作为 runner-runtime bridge。

## Tools

当前 model tools 已实现：

- `workflow_list`
- `workflow_describe`
- `workflow_start`
- `workflow_status`
- `workflow_resume`
- `workflow_abort`

其中 `workflow_list`、`workflow_describe` 和 init context 使用同一套 registry 数据。`workflow_start/status/resume/abort` 当前管理 durable `WorkflowRun`，会返回 `runId`、workflow metadata、状态、runnerStatus、inputs、revision、时间戳和 snapshot path。`start/resume/abort` 会记录 workflow progress，并通过 typed `EventMsg -> ThreadItem` lifecycle 投影为客户端 `ThreadItem::WorkflowRunProgress`；迁移期可继续用 `ResponseItem::WorkflowRunProgress` 作为模型历史兼容记录。

当前 runner 会 snapshot workflow 目录到 `$CODEX_HOME/workflow-runs/<runId>`，写入 `@codex/workflow` shim，并启动 Node 子进程执行 TypeScript entry。`runnerStatus` 使用 `runner_starting`、`runner_active`、`runner_resuming`、`completed`、`failed`、`aborted` 表达当前能力边界。通过 `workflow_start` / `workflow_resume` model tool 启动时，runner 会绑定当前 turn 的 runtime bridge；app-server v2 直连 workflow RPC 不绑定这层 bridge。

## TypeScript Runner

Rust 生成 bootstrap runner 并通过 Node 执行 snapshot 中的 workflow entry：

```text
node runner.mjs '<run-input-json>'
```

runner 负责：

- import snapshot 中的 workflow entry。
- 校验默认导出的 definition object。
- 创建 runtime `wf` object。
- 调用 `definition.run(wf)`。
- 将最终 `{ output }` JSON 写到 stdout，Rust 从 stdout 最后一条可解析输出中读取结果。

当前 `wf.Agent`、`agent.wait()` 和 `agent.followup()` 已通过 runner-runtime bridge 调用真实 MultiAgent V2 runtime。`wf.shell()` 仍不调用 unified exec：第一阶段会返回明确 unsupported RPC error，避免绕过 exec permission、hook、environment 和 typed command item lifecycle。

用户 workflow 只导出 definition：

```ts
export default defineWorkflow({
  id: "feature-dev",
  version: "0.1.0",
  staticGraph: {},
  async run(wf) {}
});
```

## Runner-runtime Bridge

`wf.Agent`、`agent.wait()` 和 `agent.followup()` 接入真实 runtime 时，第一阶段使用父子进程 stdio line-delimited JSON。该 bridge 只由 `workflow_start`/`workflow_resume` tool 启动的 runner 绑定当前 `Session`、`TurnContext` 和当前 agent path；app-server `workflow/*` RPC 不绑定这层 bridge。

```text
Node stdout -> Rust host 读取 runner frames
Node stdin  <- Rust host 写入 RPC responses
Node stderr -> runner 日志
```

stdout frame：

```json
{"type":"rpc","id":1,"method":"agent.spawn","params":{}}
{"type":"event","event":{}}
{"type":"output","output":{}}
```

stdin frame：

```json
{"type":"rpc_result","id":1,"result":{}}
{"type":"rpc_error","id":1,"error":"unsupported"}
```

启用 bridge 后 stdout 只允许这些 protocol frames；runner 应将 `console.log/info/warn/error` 重定向到 stderr，避免日志和协议混淆。当前实现尚未启用这条 RPC 通道，stdout 仍用于最终 `{ output }` JSON。

第一阶段 RPC method：

```text
agent.spawn
agent.followup
agent.wait
shell.exec
```

第一阶段能力边界：

- `agent.spawn` 映射到 MultiAgent V2 `spawn_agent`，复用 canonical typed spawn lifecycle，返回 canonical agent path。绑定会写入 `WorkflowRun.bindings`，同一 run 内重复 `Agent(id)` 和 resume 后同 id 会返回已有 binding，不重复 spawn。
- `agent.followup` 映射到 MultiAgent V2 `followup_task`，复用 typed inter-agent communication，不从 raw marker、assistant text 或 legacy envelope 解析。
- `agent.wait` 复用 MultiAgent V2 wait 的 pending mailbox/status watch 语义，不通过 raw marker、assistant text 或 legacy envelope 解析。
- `wf.shell` 后续应映射到真实 `exec_command`/unified exec 能力，并沿用权限、hook、environment 和 typed command item lifecycle。第一阶段尚未安全接入时返回明确 unsupported typed error，不得继续无效果占位。

真实权限、cwd、approval、agent primitive 都在 Rust host 中执行，workflow 脚本不直接绕过现有策略。workflow run progress 继续通过 typed `EventMsg -> ThreadItem` live/history 路径展示；迁移期可保留 `ResponseItem::WorkflowRunProgress` 模型历史记录，不新增 raw marker 或 assistant message JSON 解析路径。

## Resume

Resume 不恢复 Node 调用栈。app-server 重新启动 runner，runner 重新执行 snapshot bundle：

- `wf.Agent(id)` 查询已有 binding，返回已有 agent handle。
- `agent.wait()` 查询已有 agent status。
- runtimeGraph 从 workflow run state 恢复。
- staticGraph 从 snapshot manifest 恢复。

启动 workflow 时应 snapshot：

```text
workflow_runs/<run-id>/
  WORKFLOW.md
  workflow.ts
  workflow.bundle.mjs
  manifest.json
  state.json
```

## 与 Item 重构的关系

当前已定义最小 workflow progress typed item：

- `ResponseItem::WorkflowRunProgress`
- `ThreadItem::WorkflowRunProgress`
- `WorkflowRunProgressEvent { runId, workflowId, status, runnerStatus, kind, message, updatedAt }`

该路径必须继续复用 `codex-rs/app-server-protocol/src/protocol/response_item_projection.rs`，不要新增 raw response item、message marker 或 assistant JSON 解析分支。

当前仍可并行完成的部分：

- TypeScript authoring API 草案。
- runner 与 host RPC 边界。
- 示例 workflow。

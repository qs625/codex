# Dynamic Workflow Registry 与 Runner 设计

## 目标

本文补充 `dynamic-workflow.md` 中可并行推进的实现边界，聚焦不依赖 `ResponseItem -> ThreadItem` 大重构的部分：

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
  workflow.json
  workflow.ts
  README.md
```

单文件 workflow 可以作为后续便捷入口，但第一版推荐只支持目录形式，减少 metadata 和 entry 解析歧义。

## Registry 规则

扫描顺序：

1. `$CODEX_HOME/workflows/*/workflow.json`
2. `.codex/workflows/*/workflow.json`

冲突规则：

- project workflow 覆盖 home workflow。
- 同一来源出现重复 `id` 时，标记为 invalid，不注入 init context。
- `workflow.json` 的 `id` 必须与目录名一致。
- `workflow.json.entry` 必须指向同目录下的 TypeScript 文件。

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

Session 初始化时只注入 workflow 摘要，不注入完整 `workflow.ts`。

建议格式：

```text
## Available Workflows

Workflows are scripted, resumable multi-agent procedures. Use `workflow_list`
or `workflow_describe` when the user asks for a structured workflow and the task
matches one of the entries below. Use `workflow_start`, `workflow_status`,
`workflow_resume`, and `workflow_abort` to manage a workflow run.

- feature-dev (project)
  Description: 按调研、实现、review/fix、验证流程开发功能。
  Use when: 新功能、复杂 bugfix、需要 owner/reviewer 流程。
  Inputs: objective, cwd
  Inspect: workflow_describe({"workflow": "feature-dev"})
```

注入规则：

- 每个 workflow 摘要应控制在数行内。
- 不注入 TypeScript 源码、完整 staticGraph 或 README。
- 详细信息通过 `workflow_describe` 或 `workflow_status` 按需读取。

## Tools

当前已实现：

- `workflow_list`
- `workflow_describe`
- `workflow_start`
- `workflow_status`
- `workflow_resume`
- `workflow_abort`

其中 `workflow_list`、`workflow_describe` 和 init context 使用同一套 registry 数据。`workflow_start/status/resume/abort` 当前管理 session 内存态 `WorkflowRun`，会返回 `runId`、workflow metadata、状态、runnerStatus、inputs、revision 和时间戳。`start/resume/abort` 会记录 typed `WorkflowRunProgress`，并通过共享 `ResponseItem -> ThreadItem` projector 投影为客户端 `ThreadItem::WorkflowRunProgress`。

当前 run control 是控制面骨架，不执行 TypeScript entry，不创建 Node 子进程，也不 snapshot bundle。`runnerStatus` 使用 `control_plane_started`、`control_plane_resumed`、`aborted` 明确表达当前能力边界。完整 TS runner、durable snapshot、agent binding 和 app-server v2 直连 RPC 仍是后续阶段。

## TypeScript Runner

Node 直接执行 Codex 提供的 runner，而不是直接执行用户 workflow：

```text
node workflow-runner.mjs --mode run --bundle workflow.bundle.mjs --run-id wf_123
```

runner 负责：

- import workflow bundle。
- 校验默认导出的 definition object。
- 创建 runtime `wf` object。
- 调用 `definition.run(wf)`。
- 通过 stdio NDJSON RPC 请求 app-server 执行真实能力。

用户 workflow 只导出 definition：

```ts
export default defineWorkflow({
  id: "feature-dev",
  version: "0.1.0",
  staticGraph: {},
  async run(wf) {}
});
```

## RPC 边界

runner 与 app-server 使用父子进程 stdio：

```text
Node stdout -> app-server 读取 RPC request
Node stdin  <- app-server 写入 RPC response
Node stderr -> runner 日志
```

stdout 只允许 NDJSON RPC。runner 应将 `console.log/info/warn/error` 重定向到 stderr，避免破坏协议。

第一版 RPC method 草案：

```text
workflow.graph.patch
workflow.runner.suspend
workflow.runner.complete
agent.getOrCreate
agent.wait
agent.followup
shell.run
artifact.write
```

真实权限、cwd、approval、agent primitive 都在 app-server/Rust host 中执行，workflow 脚本不直接绕过现有策略。

## Resume

Resume 不恢复 Node 调用栈。app-server 重新启动 runner，runner 重新执行 snapshot bundle：

- `wf.Agent(id)` 查询已有 binding，返回已有 agent handle。
- `agent.wait()` 查询已有 agent status。
- runtimeGraph 从 workflow run state 恢复。
- staticGraph 从 snapshot manifest 恢复。

启动 workflow 时应 snapshot：

```text
workflow_runs/<run-id>/
  workflow.json
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

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
matches one of the entries below.

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

当前已实现的 registry 阶段只暴露：

- `workflow_list`
- `workflow_describe`

runner 阶段再暴露：

第一版建议暴露：

- `workflow_start`
- `workflow_status`
- `workflow_resume`
- `workflow_abort`

其中 `workflow_list` 和 init context 使用同一套 registry 数据。

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

本设计文档不定义最终 workflow `ResponseItem` / `ThreadItem` shape。该部分应等待 typed `ResponseItem -> ThreadItem` 主路径重构完成后再实现。

当前可并行完成的部分：

- registry 发现规则。
- init context 摘要格式。
- workflow 文件布局。
- TypeScript authoring API 草案。
- runner 与 host RPC 边界。
- 示例 workflow。

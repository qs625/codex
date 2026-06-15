# Feature Development Workflow

## 用途

`feature-dev` 用于把常见工程开发流程固定为可 resume 的多 agent workflow：

```text
Research -> Implement -> Review/Fix -> Verify
```

适用于：

- 新功能开发。
- 复杂 bugfix。
- 需要 owner 和 reviewer 协作的重构或行为调整。
- 希望客户端展示 workflow 预期流程和实际 agent 执行节点的任务。

## 输入

- `objective`：要完成的开发目标。
- `cwd`：执行 workflow 的仓库或 worktree 路径。

## Agent Session

workflow 会创建长期 agent session：

- `explorer`：只读调研。
- `owner`：负责实现和修复 review findings。
- `reviewer`：同一 workflow run 内复用一个 reviewer session；修复后通过 followup 请求同一 reviewer 复审。
- `tester`：根据 reviewer 结论执行必要验证；Rust/Cargo 命令必须按 `AGENTS.md` 串行执行。

`Agent(id)` 应在 resume 时绑定回已有 agent session，不重复 spawn。

当前 Rust runner 已能执行该 TypeScript entry，并会把 `Agent`、`followup`、`shell` 请求记录到 run output events 中；真实 subagent spawn/followup/shell 执行仍未接入。这个 workflow 目前可作为 durable runner、snapshot、resume、abort 和静态图 authoring 的端到端示例，不代表已经会自动完成真实工程开发。

## Static Graph

静态图只展示高层骨架：

```text
Research -> Implement -> Review/Fix -> Verify
```

运行时图会把实际 agent/thread 节点挂到对应 stage 下，例如：

```text
research: explorer
implement: owner
review_fix: reviewer
verify: tester
```

## Resume

Resume 时重新执行 `workflow.ts`，但 `wf.Agent(id)` 返回已有 agent session handle。

重复 followup 由 agent session 根据上下文自行判断是否需要动作。非 agent 的高风险副作用应使用显式 durable step 或 approval。

## 当前状态

Dynamic Workflow runner 已支持：

- 发现 project/home workflow，并按 project 覆盖 home。
- 校验 `workflow.json.id` 与目录名一致、entry 是目录内 TypeScript 文件。
- snapshot workflow 目录到 `$CODEX_HOME/workflow-runs/<runId>/`。
- 启动 Node runner 执行 snapshot entry。
- 持久化 `run.json`，支持 status/resume/abort 查询和恢复。
- 通过 typed `WorkflowRunProgress` 展示 start/resume/abort 进度。

尚未支持真实 MultiAgent runtime callback，因此 `wf.Agent` 和 `wf.shell` 在当前 runner 中是结构化占位 API。

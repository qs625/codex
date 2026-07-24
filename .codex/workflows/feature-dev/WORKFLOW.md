---
id: feature-dev
name: Feature Development
description: 按调研、实现、review/fix、验证流程开发功能。
entry: workflow.ts
version: "0.1.0"
when_to_use:
  - 用户要求开发新功能
  - 用户要求修复复杂 bug
  - 需要多 agent 协作、review 和验证
  - 需要把单次 workflow 的 Research/Implement/Review/Verify 流程固定下来并支持 resume
inputs:
  objective:
    type: string
    description: 要完成的开发目标
  cwd:
    type: string
    description: 执行 workflow 的 checkout 路径，只能是三个固定开发 checkout 之一：~/Projects/my-codex-dev、~/Projects/my-codex-dev-2、~/Projects/my-codex-dev-3
---
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
- `cwd`：执行 workflow 的 checkout 路径，只能是三个固定开发 checkout 之一：`~/Projects/my-codex-dev`、`~/Projects/my-codex-dev-2`、`~/Projects/my-codex-dev-3`；不要使用主 checkout `~/Projects/my-codex` 承载开发任务，也不要为 workflow 创建额外 checkout 或开发目录。

## Agent Session

workflow 会在当前 workflow run 内创建并复用 agent session：

- `explorer`：只读调研。
- `owner`：负责实现和修复 review findings。
- `reviewer`：同一 workflow run 内复用一个 reviewer session；修复后通过 followup 请求同一 reviewer 复审。
- Verify 阶段不创建或复用 tester agent。review 通过后，workflow 把验证要求交给 `owner`，由 `owner` 按 `AGENTS.md` 在所属 checkout 内自行串行运行必要测试和构建，并等待 owner 最终交付。

`Agent(id)` 应在同一 workflow run 的 resume 时绑定回已有 agent session，不重复 spawn。
这里的 `id` 是 workflow logical stage/binding id，不是 agent canonical path 或 name。runner 会把 `workflowRunId + stageId` 映射到实际 spawned agent path 并持久化 binding；workflow 脚本只引用 `explorer`、`owner`、`reviewer` 这类稳定 stage id，不手写或推导 canonical path。

注意：PM 的常规开发调度使用三个固定开发 checkout owner：`owner_dev`、`owner_dev_2`、`owner_dev_3`；主 checkout 的 `owner_main` 只承接 refactor/performance 全局独占任务。当前 workflow SDK 的 `Agent(id)` 绑定范围是单个 workflow run，因此 `feature-dev` 不适合作为 PM 固定 owner 池的派发入口；PM 不应为了普通开发任务启动 workflow 来绕过固定 owner 复用规则。只有用户明确要求测试或执行 dynamic workflow 时，才使用本 workflow。

当前 Rust runner 已能执行该 TypeScript entry，并在通过 `workflow_start` / `workflow_resume` model tool 启动时把 `Agent`、`followup`、`event.poll` 请求桥接到真实 MultiAgent V2 runtime。`Agent(id)` 的 binding 会持久化到 workflow run snapshot，resume 后同 id 返回已有 session，不重复 spawn。`wf.shell` 尚未安全接入 unified exec，调用时会返回明确 unsupported error。

`feature-dev` 的四个 agent stage 都需要显式 agent type，因此脚本使用 `fork_turns: "none"` 创建独立上下文的 subagent。MultiAgent V2 默认 `fork_turns` 是 full history；full-history fork 必须继承父 thread 的 agent type、model 和 reasoning effort，不能同时传 `type` / `agent_type`、`model` 或 `reasoning_effort`。

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
verify: owner self-test
```

## Resume

Resume 时重新执行 `workflow.ts`，但 `wf.Agent(id)` 返回已有 agent session handle。

重复 followup 由 agent session 根据上下文自行判断是否需要动作。非 agent 的高风险副作用应使用显式 durable step 或 approval。

## 当前状态

Dynamic Workflow runner 已支持：

- 发现 project/home workflow，并按 project 覆盖 home。
- 校验 `WORKFLOW.md` frontmatter 中的 `id` 与目录名一致、`entry` 是目录内 TypeScript 文件。
- snapshot workflow 目录到 `$CODEX_HOME/workflow-runs/<runId>`。
- 启动 Node runner 执行 snapshot entry。
- 持久化 `run.json`，支持 status/resume/abort 查询和恢复。
- 通过 typed `WorkflowRunProgress` 展示 start/resume/abort 进度。

已支持 `wf.Agent`、`agent.followup()` 和 `wf.pollEvent()` 的真实 MultiAgent runtime callback。`wf.pollEvent()` 是当前推荐等待入口，会桥接到 provider-neutral `event.poll`，参数始终是空对象；它不是 target-specific wait，也不接受 agent id、agent path 或 command id。一次 poll 可能只是 timeout、status update、command output、command exit 或其他 pending event，不代表某个 agent 已完成。需要把一个阶段的结果交给下一阶段时，workflow 必须循环调用 `wf.pollEvent()`，扫描 `events` 中 `type === "inter_agent_communication"` 且 `operation === "childCompletion"` 的 typed payload，并按 `communication.author` 与 agent binding 匹配目标 agent；`event` 只是与 `sourceHint` 同源的首个 payload 快捷字段。由于 pending input 在模型请求构造前不会被 poll 消费，脚本应记录已见 completion key，避免旧 completion 留队时重复处理。最终文本来自 `event.communication.status` 或 `event.communication.content`。workflow 不应读取不存在的 `summary`、`blockingFindings` 等临时字段。

`agent.wait()` 仍作为 runtime 兼容 alias 保留，旧 workflow 可以继续运行；新 workflow 应直接使用 `wf.pollEvent()`，并在脚本层根据 payload 来源过滤所需 agent。文档和 brief 不应继续推荐旧的 target-specific wait surface；长命令由执行它的 owner 通过 `poll_event` 等待 command output / command exit 事件并在交付中报告。Verify 阶段由 owner 在所属 checkout 自行运行命令，workflow 不创建共享 tester。`wf.shell` 仍是明确 unsupported，后续接入前不得绕过 exec permission、hook 或 typed command lifecycle。

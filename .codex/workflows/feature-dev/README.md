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
- `reviewer-<n>`：每轮 review 创建一个 reviewer。

`Agent(id)` 应在 resume 时绑定回已有 agent session，不重复 spawn。

## Static Graph

静态图只展示高层骨架：

```text
Research -> Implement -> Review/Fix -> Verify
```

运行时图会把实际 agent/thread 节点挂到对应 stage 下，例如：

```text
research: explorer
implement: owner
review_fix: reviewer-0, reviewer-1
```

## Resume

Resume 时重新执行 `workflow.ts`，但 `wf.Agent(id)` 返回已有 agent session handle。

重复 followup 由 agent session 根据上下文自行判断是否需要动作。非 agent 的高风险副作用应使用显式 durable step 或 approval。

## 当前状态

这是 dynamic workflow runner 尚未实现前的 project workflow 模板。它用于固定文件布局、metadata、staticGraph 和 authoring API 方向。

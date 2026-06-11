---
name: create-workflow
description: '创建或更新 Codex Dynamic Workflow。用于用户要求新增 workflow、编写 TypeScript workflow 脚本、设计 workflow graph、注册 project/home workflow、补充 workflow init context 或 workflow 使用说明时。'
---

# Create Workflow

## 目标

创建或更新可被 Codex 发现和执行的 Dynamic Workflow。Workflow 是脚本化的多 agent 编排能力，不是普通 instruction 文档；它应通过 TypeScript definition 描述静态流程骨架，并在运行时创建长期 agent session。

## 何时使用

- 用户要求创建新的 workflow。
- 用户要求把重复开发流程、review 流程、测试流程或多 agent 协作流程脚本化。
- 用户要求修改 `.codex/workflows/` 或 `$CODEX_HOME/workflows/` 下的 workflow。
- 用户要求补充 workflow 的 metadata、staticGraph、README 或 init context 摘要。
- 用户要求把现有手动 PM/owner/reviewer 流程沉淀为可 resume 的 workflow。

## 放置位置

优先创建 project workflow：

```text
.codex/workflows/<workflow-id>/
  workflow.json
  workflow.ts
  README.md
```

需要跨项目复用时，创建 home workflow：

```text
$CODEX_HOME/workflows/<workflow-id>/
  workflow.json
  workflow.ts
  README.md
```

同名 workflow 的解析优先级为 project 高于 home。不要在同一来源内创建重复 `id`。

## 设计原则

- `workflow.ts` 默认导出 `defineWorkflow(...)` 的 definition object。
- `staticGraph` 只表达高层流程骨架，用于客户端预览，不要求精确列出所有动态节点。
- 动态执行时由 runtimeGraph 填充实际 agent/thread/shell/gate 节点。
- workflow 中的 agent 应作为长期 session handle 使用，例如 `wf.Agent("owner", ...)`。
- `Agent(id)` 必须能 resume 到已有 agent session，不应重复 spawn。
- branch、loop、parallel 使用普通 TypeScript 表达；不要为了可视化而发明复杂 DSL。
- 所有外部系统能力都应通过 `wf` runtime API 回调 host，不要在 workflow 中绕过 Codex permission、agent primitive 或 shell policy。
- workflow 新增结构化进展时应走 typed `ResponseItem -> ThreadItem` 展示路径，不要依赖 raw message 或从文本特殊解析。

## workflow.json

为每个 workflow 提供简短 metadata，供 registry 和 init context 使用：

```json
{
  "id": "feature-dev",
  "name": "Feature Development",
  "description": "按调研、实现、review/fix、验证流程开发功能",
  "entry": "workflow.ts",
  "version": "1.0.0",
  "when_to_use": [
    "用户要求开发新功能",
    "用户要求修复复杂 bug",
    "需要多 agent 协作、review 和验证"
  ],
  "inputs": {
    "objective": {
      "type": "string",
      "description": "要完成的任务目标"
    },
    "cwd": {
      "type": "string",
      "description": "执行 workflow 的仓库或 worktree 路径"
    }
  }
}
```

## workflow.ts 模板

```ts
import { defineWorkflow } from "@codex/workflow";

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
    const explorer = await wf.Agent("explorer", {
      parent: "research",
      type: "explorer",
      cwd: wf.inputs.cwd,
      message: `只读调研任务：${wf.inputs.objective}`
    });

    const research = await explorer.wait();

    const owner = await wf.Agent("owner", {
      parent: "implement",
      type: "feature-owner",
      cwd: wf.inputs.cwd,
      message: `根据调研实现任务：${wf.inputs.objective}\n\n调研结果：${research.summary}`
    });

    let implementation = await owner.wait();

    for (let i = 0; i < 3; i += 1) {
      const reviewer = await wf.Agent(`reviewer-${i}`, {
        parent: "review_fix",
        type: "code-review",
        cwd: wf.inputs.cwd,
        message: `审查 owner 的实现，重点找阻塞问题。\n\n实现结果：${implementation.summary}`
      });

      const review = await reviewer.wait();
      if (!review.blockingFindings || review.blockingFindings.length === 0) {
        break;
      }

      await owner.followup(`修复 review findings：\n${JSON.stringify(review.blockingFindings)}`);
      implementation = await owner.wait();
    }

    await wf.shell({
      parent: "verify",
      command: "rtk cargo test -p <crate>",
      cwd: wf.inputs.cwd
    });
  }
});
```

## README.md 内容

每个 workflow 应包含简短说明：

- workflow 做什么。
- 什么时候使用。
- 需要哪些输入。
- 会创建哪些主要 agent session。
- staticGraph 的高层流程。
- resume 行为和注意事项。
- 可能的风险和人工确认点。

## 创建流程

1. 明确 workflow 目标、输入、适用场景和是否 project/home 级别。
2. 创建 workflow 目录和 `workflow.json`。
3. 编写 `workflow.ts`，使用 `defineWorkflow`、`staticGraph` 和长期 `wf.Agent` handle。
4. 编写 `README.md`，用中文说明使用方式。
5. 检查 `workflow.json` 与 `workflow.ts` 的 `id`、`version`、entry 是否一致。
6. 如果 workflow 会出现在 session init context，确保 description 和 `when_to_use` 简短、可检索。
7. 如果修改了项目级 workflow 规则，同步更新 `AGENTS.md` 或说明无需更新。

## 质量检查

- workflow id 使用小写字母、数字和连字符。
- `staticGraph` 节点 id 稳定，不使用运行时随机值。
- 动态 agent 节点通过 `parent` 挂到 staticGraph 节点下。
- 不从 TypeScript AST 推断 graph；graph 由 definition 显式声明。
- 不把完整 workflow 源码注入 init context。
- 不让 workflow 绕过现有权限、approval、agent session 和 shell 执行策略。
- 非 agent 的高风险副作用需要 explicit gate、approval 或 durable step。

## 相关设计

更多架构细节见仓库内 `spec/dynamic-workflow.md`。

可参考项目内示例 workflow：

- `.codex/workflows/feature-dev/workflow.json`
- `.codex/workflows/feature-dev/workflow.ts`
- `.codex/workflows/feature-dev/README.md`

Registry、init context 和 runner 边界见 `spec/workflow-registry-and-runner.md`。

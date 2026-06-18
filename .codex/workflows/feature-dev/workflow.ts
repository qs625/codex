import { defineWorkflow } from "@codex/workflow";

function statusKind(status) {
  if (typeof status === "string") {
    return status;
  }
  if (!status || typeof status !== "object") {
    return "unknown";
  }
  const keys = Object.keys(status);
  return keys.length > 0 ? keys[0] : "unknown";
}

function statusMessage(status) {
  if (!status || typeof status !== "object") {
    return "";
  }
  const kind = statusKind(status);
  const value = status[kind];
  return typeof value === "string" ? value : "";
}

function isFinalWaitResult(result) {
  const kind = statusKind(result?.status);
  return (
    result?.message_operation === "child_completion" ||
    result?.reason === "final_status" ||
    kind === "completed" ||
    kind === "errored" ||
    kind === "shutdown" ||
    kind === "not_found"
  );
}

function waitResultText(result) {
  const statusText = statusMessage(result?.status);
  if (statusText.trim()) {
    return statusText;
  }
  if (typeof result?.message_excerpt === "string" && result.message_excerpt.trim()) {
    return result.message_excerpt;
  }
  return JSON.stringify(result);
}

async function waitForAgentResult(agent, label) {
  for (;;) {
    const result = await agent.wait();
    if (!isFinalWaitResult(result)) {
      continue;
    }

    const kind = statusKind(result?.status);
    if (kind === "errored" || kind === "shutdown" || kind === "not_found") {
      throw new Error(`${label} 未成功完成：${waitResultText(result)}`);
    }

    return {
      raw: result,
      text: waitResultText(result)
    };
  }
}

function reviewHasBlockingFindings(reviewText) {
  const normalized = reviewText.replace(/\r\n/g, "\n");
  return !(
    normalized.includes("状态：\n通过") ||
    normalized.includes("状态：通过") ||
    normalized.includes("结论：\n可继续") ||
    normalized.includes("结论：可继续")
  );
}

export default defineWorkflow({
  id: "feature-dev",
  version: "0.1.0",
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
      fork_turns: "none",
      cwd: wf.inputs.cwd,
      message: `只读调研任务：${wf.inputs.objective}`
    });

    const research = await waitForAgentResult(explorer, "Research");

    const owner = await wf.Agent("owner", {
      parent: "implement",
      type: "feature-owner",
      fork_turns: "none",
      cwd: wf.inputs.cwd,
      message: `根据调研结果实现任务：${wf.inputs.objective}\n\n调研结果：${research.text}`
    });

    let implementation = await waitForAgentResult(owner, "Implement");

    const reviewer = await wf.Agent("reviewer", {
      parent: "review_fix",
      type: "code-review",
      fork_turns: "none",
      cwd: wf.inputs.cwd,
      message: `审查 owner 的实现，优先找阻塞问题、行为回归和测试缺口。只做代码评审，不执行命令，也不委派 tester。\n\n实现结果：${implementation.text}`
    });

    const maxReviewIterations = 3;
    let finalReview = null;
    for (let iteration = 0; iteration < maxReviewIterations; iteration += 1) {
      const review = await waitForAgentResult(reviewer, "Review");
      finalReview = review;
      if (!reviewHasBlockingFindings(review.text)) {
        break;
      }

      if (iteration === maxReviewIterations - 1) {
        throw new Error(
          `review 仍有阻塞问题，已达到最大复审次数：${review.text}`,
        );
      }

      await owner.followup(
        `修复 reviewer 发现的问题。修复后按 owner 交付格式总结改动、文件范围和验证计划；review 通过前不要执行 Rust/Cargo 测试、构建、格式化或 lint。\n\nreview 结论：\n${review.text}`,
      );
      implementation = await waitForAgentResult(owner, "Implement fix");
      await reviewer.followup(
        `复审 owner 修复后的实现，继续优先找阻塞问题、行为回归和测试缺口。只做代码评审，不执行命令，也不委派 tester。\n\n最新实现结果：${implementation.text}`,
      );
    }

    wf.emit({
      type: "verifyHandoff",
      checkout: wf.inputs.cwd,
      owner: owner.binding?.agentPath ?? null
    });
    await owner.followup(
      `reviewer 已无阻塞问题。现在进入 Verify 阶段：请你作为 owner 在当前 checkout 自行串行运行必要验证命令，并在最终交付中写清命令、工作目录、退出状态和关键输出。\n\n默认验证只包含修改模块的单元测试/最小 crate 测试，以及与入口匹配的 binary 编译验证；只涉及 app-server、runtime、protocol 或 root-worker 后端启动路径时使用 ${wf.inputs.cwd}/codex-rs 下的 cargo build -p codex-app-server --bin codex-app-server，只有确实改到 CLI/TUI 或 CLI app-server 子命令包装时才使用 cargo build -p codex-cli。所有 shell 命令必须带 rtk 前缀；长命令用 command_wait 等待完成通知。不得创建或复用 tester agent。\n\n最终 review 结论：\n${finalReview.text}`,
    );
    const verification = await waitForAgentResult(owner, "Verify");
    return {
      research: research.text,
      implementation: implementation.text,
      finalReview: finalReview.text,
      verification: verification.text
    };
  }
});

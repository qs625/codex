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

function testerRequestTemplate(wf, finalReviewText) {
  return JSON.stringify(
    {
      type: "rust_cargo_validation_request",
      request_id: `${wf.runId}-verify`,
      requested_by: "<owner canonical path>",
      report_to: "<owner canonical path>",
      worktree: wf.inputs.cwd,
      branch: "<当前分支>",
      commands: [
        {
          id: "<最小验证命令 id>",
          exec_command: {
            cmd: "rtk <需要执行的 Rust/Cargo 命令>",
            workdir: `${wf.inputs.cwd}/codex-rs`,
            initial_wait_ms: 30000,
            notify_on: "exit",
            max_output_tokens: 20000
          }
        }
      ],
      notes: `reviewer 最终结论：${finalReviewText}`
    },
    null,
    2
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
      message: `审查 owner 的实现，优先找阻塞问题、行为回归和测试缺口。只做代码评审，不执行命令，也不 followup tester。\n\n实现结果：${implementation.text}`
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
        `修复 reviewer 发现的问题。修复后按 owner 交付格式总结改动、文件范围和验证计划；不要直接执行 Rust/Cargo 测试、构建、格式化或 lint。\n\nreview 结论：\n${review.text}`,
      );
      implementation = await waitForAgentResult(owner, "Implement fix");
      await reviewer.followup(
        `复审 owner 修复后的实现，继续优先找阻塞问题、行为回归和测试缺口。只做代码评审，不执行命令，也不 followup tester。\n\n最新实现结果：${implementation.text}`,
      );
    }

    wf.emit({
      type: "verifyHandoff",
      tester: "/root/my_codex_pm/rust_cargo_tester",
      owner: owner.binding?.agentPath ?? null
    });
    await owner.followup(
      `reviewer 已无阻塞问题。现在进入 Verify 阶段：请你作为 owner 按项目固定 tester 协议，自行通过 followup_task 向 /root/my_codex_pm/rust_cargo_tester 发送 rust_cargo_validation_request JSON，等待 tester 回传结果后再完成最终交付。\n\n不得创建新的 tester agent，不得发送自由文本测试请求。请求模板如下，请替换 requested_by、report_to、branch 和 commands：\n\n${testerRequestTemplate(wf, finalReview.text)}\n\n最终 review 结论：\n${finalReview.text}`,
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

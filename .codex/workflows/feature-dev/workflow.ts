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

function eventCommunication(result) {
  const event = result?.event;
  if (event?.type !== "inter_agent_communication") {
    return null;
  }
  return event.communication && typeof event.communication === "object"
    ? event.communication
    : null;
}

function resultEvents(result) {
  const events = Array.isArray(result?.events) ? [...result.events] : [];
  if (
    result?.event &&
    !events.some((event) => baseEventKey(event) === baseEventKey(result.event))
  ) {
    events.unshift(result.event);
  }
  return events;
}

function resultWithEvent(result, event) {
  return {
    ...result,
    event,
    sourceHint: result?.sourceHint ?? "child_completion"
  };
}

function baseEventKey(event) {
  const communication = event?.communication;
  return JSON.stringify({
    type: event?.type ?? null,
    author: communication?.author ?? null,
    senderThreadId: communication?.sender_thread_id ?? null,
    operation: communication?.operation ?? null,
    status: communication?.status ?? null,
    content: communication?.content ?? null
  });
}

function eventKey(event, occurrence) {
  return `${baseEventKey(event)}#${occurrence}`;
}

function eventIsFinalCompletion(event) {
  const communication = event?.communication;
  const operation = communication?.operation;
  return (
    event?.type === "inter_agent_communication" &&
    (operation === "childCompletion" || operation === "child_completion") &&
    Boolean(communication?.status)
  );
}

function eventMatchesAgent(event, agent) {
  const communication = event?.communication;
  if (!communication) {
    return true;
  }
  const agentPath = agent?.binding?.agentPath;
  if (!agentPath) {
    return true;
  }
  return communication.author === agentPath;
}

function resultStatus(result) {
  return eventCommunication(result)?.status ?? result?.status;
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
  const communication = eventCommunication(result);
  const kind = statusKind(resultStatus(result));
  return (
    (result?.sourceHint === "child_completion" && Boolean(communication?.status)) ||
    result?.message_operation === "child_completion" ||
    result?.reason === "final_status" ||
    kind === "completed" ||
    kind === "errored" ||
    kind === "shutdown" ||
    kind === "not_found"
  );
}

function waitResultText(result) {
  const statusText = statusMessage(resultStatus(result));
  if (statusText.trim()) {
    return statusText;
  }
  const content = eventCommunication(result)?.content;
  if (typeof content === "string" && content.trim()) {
    return content;
  }
  if (typeof result?.message_excerpt === "string" && result.message_excerpt.trim()) {
    return result.message_excerpt;
  }
  return JSON.stringify(result);
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function waitForAgentResult(wf, agent, label, seenEvents) {
  for (;;) {
    const result = await wf.pollEvent();
    let sawUnseenCompletion = false;
    const occurrenceByBaseKey = new Map();
    for (const event of resultEvents(result)) {
      if (!eventIsFinalCompletion(event)) {
        continue;
      }
      const baseKey = baseEventKey(event);
      const occurrence = occurrenceByBaseKey.get(baseKey) ?? 0;
      occurrenceByBaseKey.set(baseKey, occurrence + 1);
      const key = eventKey(event, occurrence);
      if (seenEvents.has(key)) {
        continue;
      }
      sawUnseenCompletion = true;
      seenEvents.add(key);
      if (!eventMatchesAgent(event, agent)) {
        continue;
      }
      const matchedResult = resultWithEvent(result, event);
      const kind = statusKind(resultStatus(matchedResult));
      if (kind === "errored" || kind === "shutdown" || kind === "not_found") {
        throw new Error(`${label} 未成功完成：${waitResultText(matchedResult)}`);
      }

      return {
        raw: matchedResult,
        text: waitResultText(matchedResult)
      };
    }

    if (resultEvents(result).length === 0 && isFinalWaitResult(result)) {
      const kind = statusKind(resultStatus(result));
      if (kind === "errored" || kind === "shutdown" || kind === "not_found") {
        throw new Error(`${label} 未成功完成：${waitResultText(result)}`);
      }

      return {
        raw: result,
        text: waitResultText(result)
      };
    }

    if (!sawUnseenCompletion && resultEvents(result).length > 0) {
      await sleep(250);
      continue;
    }
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
    const seenEvents = new Set();
    const explorer = await wf.Agent("explorer", {
      parent: "research",
      type: "explorer",
      fork_turns: "none",
      cwd: wf.inputs.cwd,
      message: `只读调研任务：${wf.inputs.objective}`
    });

    const research = await waitForAgentResult(wf, explorer, "Research", seenEvents);

    const owner = await wf.Agent("owner", {
      parent: "implement",
      type: "feature-owner",
      fork_turns: "none",
      cwd: wf.inputs.cwd,
      message: `根据调研结果实现任务：${wf.inputs.objective}\n\n调研结果：${research.text}`
    });

    let implementation = await waitForAgentResult(wf, owner, "Implement", seenEvents);

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
      const review = await waitForAgentResult(wf, reviewer, "Review", seenEvents);
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
      implementation = await waitForAgentResult(wf, owner, "Implement fix", seenEvents);
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
      `reviewer 已无阻塞问题。现在进入 Verify 阶段：请你作为 owner 在当前 checkout 自行串行运行必要验证命令，并在最终交付中写清命令、工作目录、退出状态和关键输出。\n\n默认验证只包含修改模块的单元测试/最小 crate 测试，以及与入口匹配的 binary 编译验证；只涉及 app-server、runtime、protocol 或 root-worker 后端启动路径时使用 ${wf.inputs.cwd}/codex-rs 下的 cargo build -p codex-app-server --bin codex-app-server，只有确实改到 CLI/TUI 或 CLI app-server 子命令包装时才使用 cargo build -p codex-cli。所有 shell 命令必须带 rtk 前缀；长命令通过 poll_event 等待 command output 或 command exit 事件。不得创建或复用 tester agent。\n\n最终 review 结论：\n${finalReview.text}`,
    );
    const verification = await waitForAgentResult(wf, owner, "Verify", seenEvents);
    return {
      research: research.text,
      implementation: implementation.text,
      finalReview: finalReview.text,
      verification: verification.text
    };
  }
});

import { defineWorkflow } from "@codex/workflow";

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
      cwd: wf.inputs.cwd,
      message: `只读调研任务：${wf.inputs.objective}`
    });

    const research = await explorer.wait();

    const owner = await wf.Agent("owner", {
      parent: "implement",
      type: "feature-owner",
      cwd: wf.inputs.cwd,
      message: `根据调研结果实现任务：${wf.inputs.objective}\n\n调研结果：${research.summary}`
    });

    let implementation = await owner.wait();

    const reviewer = await wf.Agent("reviewer", {
      parent: "review_fix",
      type: "code-review",
      cwd: wf.inputs.cwd,
      message: `审查 owner 的实现，优先找阻塞问题、行为回归和测试缺口。\n\n实现结果：${implementation.summary}`
    });

    const maxReviewIterations = 3;
    let finalReview = null;
    for (let iteration = 0; iteration < maxReviewIterations; iteration += 1) {
      const review = await reviewer.wait();
      finalReview = review;
      if (!review.blockingFindings || review.blockingFindings.length === 0) {
        break;
      }

      if (iteration === maxReviewIterations - 1) {
        throw new Error(
          `review 仍有阻塞问题，已达到最大复审次数：${JSON.stringify(review.blockingFindings)}`,
        );
      }

      await owner.followup(
        `修复 review findings：\n${JSON.stringify(review.blockingFindings, null, 2)}`,
      );
      implementation = await owner.wait();
      await reviewer.followup(
        `复审 owner 修复后的实现，继续优先找阻塞问题、行为回归和测试缺口。\n\n最新实现结果：${implementation.summary}`,
      );
    }

    const tester = await wf.Agent("tester", {
      parent: "verify",
      type: "test_agent",
      cwd: wf.inputs.cwd,
      message: `根据 reviewer 最终结论执行必要验证；Rust/Cargo 命令必须按 AGENTS.md 串行执行，并回传命令结果、失败摘要和未覆盖范围。\n\n最终 review 结论：${JSON.stringify(finalReview, null, 2)}`
    });
    await tester.wait();
  }
});

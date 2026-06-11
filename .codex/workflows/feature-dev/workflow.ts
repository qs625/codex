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

    for (let iteration = 0; iteration < 3; iteration += 1) {
      const reviewer = await wf.Agent(`reviewer-${iteration}`, {
        parent: "review_fix",
        type: "code-review",
        cwd: wf.inputs.cwd,
        message: `审查 owner 的实现，优先找阻塞问题、行为回归和测试缺口。\n\n实现结果：${implementation.summary}`
      });

      const review = await reviewer.wait();
      if (!review.blockingFindings || review.blockingFindings.length === 0) {
        break;
      }

      await owner.followup(
        `修复 review findings：\n${JSON.stringify(review.blockingFindings, null, 2)}`,
      );
      implementation = await owner.wait();
    }

    await wf.shell({
      parent: "verify",
      command: "rtk cargo test -p <crate>",
      cwd: wf.inputs.cwd
    });
  }
});

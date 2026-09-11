const assert = require("node:assert/strict");
const test = require("node:test");

const {
  formatPayloadRuntimeRecoveryPrompt,
  RESTART_RECOVERY_PROMPTS,
  expectedRuntimeRestartRecoveryPrompt,
  shouldNotifyRuntimeRestartErrorOnSelf,
  shouldNotifyRuntimeRestartRecoveryOnSelf,
} = require("./restartRecoveryPrompts.cjs");

test("restart recovery prompts are centralized and Chinese", () => {
  assert.match(RESTART_RECOVERY_PROMPTS.projectRootFanout, /\p{Script=Han}/u);
  const completed = expectedRuntimeRestartRecoveryPrompt({
    requestId: "restart-1",
    phase: "completed",
  });
  assert.match(completed, /\p{Script=Han}/u);
  assert.match(completed, /restart-1/);
  assert.match(completed, /不要自动再次调用/);
});

test("restart recovery prompts preserve failed and interrupted evidence", () => {
  const failed = expectedRuntimeRestartRecoveryPrompt({
    requestId: "restart-failed",
    phase: "failed",
    error: "构建失败",
    requestedByThreadId: "origin-failed",
  });
  const interrupted = expectedRuntimeRestartRecoveryPrompt({
    requestId: "restart-interrupted",
    phase: "executing",
    requestedByThreadId: "origin-interrupted",
  });

  assert.match(failed, /\p{Script=Han}/u);
  assert.match(failed, /restart-failed/);
  assert.match(failed, /构建失败/);
  assert.match(failed, /origin-failed/);
  assert.match(interrupted, /\p{Script=Han}/u);
  assert.match(interrupted, /restart-interrupted/);
  assert.match(interrupted, /origin-interrupted/);
});

test("payload recovery formatter prefers payload-written reason", () => {
  const prompt = formatPayloadRuntimeRecoveryPrompt({
    failedReleaseId: "release-bad",
    reason: "app-server 启动失败",
    exitCode: 1,
  });

  assert.match(prompt, /\p{Script=Han}/u);
  assert.match(prompt, /release-bad/);
  assert.match(prompt, /app-server 启动失败/);
  assert.doesNotMatch(prompt, /exit code 1/);
});

test("payload recovery formatter falls back to Launcher exit evidence", () => {
  const prompt = formatPayloadRuntimeRecoveryPrompt({
    failedReleaseId: "release-bad",
    exitCode: 9,
    signal: "SIGTERM",
  });

  assert.match(prompt, /release-bad/);
  assert.match(prompt, /exit code 9/);
  assert.match(prompt, /signal SIGTERM/);
});

test("expected restart records notify /self with completed or failure details", () => {
  assert.equal(
    shouldNotifyRuntimeRestartRecoveryOnSelf({
      requestId: "restart-completed",
      requestedByThreadId: "thread-1",
      phase: "completed",
    }),
    true,
  );
  assert.equal(
    shouldNotifyRuntimeRestartRecoveryOnSelf({
      requestId: "restart-failed",
      requestedByThreadId: "thread-1",
      phase: "failed",
    }),
    true,
  );
  assert.equal(
    shouldNotifyRuntimeRestartRecoveryOnSelf({
      requestId: "restart-interrupted",
      requestedByThreadId: "thread-1",
      phase: "executing",
    }),
    true,
  );
  assert.equal(
    shouldNotifyRuntimeRestartRecoveryOnSelf({
      requestId: "restart-missing-thread",
      phase: "completed",
    }),
    false,
  );
  assert.equal(
    shouldNotifyRuntimeRestartErrorOnSelf({
      requestId: "restart-alias",
      requestedByThreadId: "thread-1",
      phase: "completed",
    }),
    true,
  );
});

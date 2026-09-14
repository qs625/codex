const assert = require("node:assert/strict");
const fs = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const {
  buildLauncherRecoveryRecordParams,
  recoverLauncherStateAtStartup,
  recoverPayloadRuntimeFailureIfPresent,
  recordLauncherRecovery,
  recordLauncherRecoveryIfPresent,
  writePayloadFailureEvidence,
} = require("./runtimeLaunchState.cjs");
const {
  formatPayloadRuntimeRecoveryPrompt,
  RESTART_RECOVERY_PROMPTS,
} = require("./restartRecoveryPrompts.cjs");
const {
  sendSelfCommandToThread,
} = require("./selfProjectThread.cjs");
const {
  createThreadAutoResumeCoordinator,
} = require("./threadAutoResume.cjs");

test("launcher recovery records durable failure evidence without mutating launcher state", async () => {
  const requests = [];
  const result = await recordLauncherRecovery({
    appServerClient: {
      async request(method, params) {
        requests.push({ method, params });
        return { recorded: true };
      },
    },
    evidence: {
      activationId: "select-1",
      releaseId: `sha256:${"a".repeat(64)}`,
      occurredAt: "2026-09-10T00:00:00.000Z",
      message: "selected runtime could not spawn",
    },
    listResult: {
      materializedSelfThreadId: "self",
      selfProjectThreadId: "self",
    },
    async subscribeThread() {
      throw new Error("already materialized");
    },
  });

  assert.equal(result, true);
  assert.deepEqual(requests, [
    {
      method: "thread/clientRecovery/record",
      params: {
        threadId: "self",
        recoveryId: `select-1:sha256:${"a".repeat(64)}`,
        activationId: "select-1",
        releaseId: `sha256:${"a".repeat(64)}`,
        occurredAt: "2026-09-10T00:00:00.000Z",
        reason: "selected runtime could not spawn",
      },
    },
  ]);
});

test("launcher recovery normalizes historical observed timestamps", () => {
  assert.equal(
    buildLauncherRecoveryRecordParams({
      activationId: "runtime-1",
      releaseId: "release-1",
      observedAtUnixMs: 0,
    }).occurredAt,
    "1970-01-01T00:00:00.000Z",
  );
});

test("payload writes a release-scoped failure reason atomically", async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "payload-failure-"));
  const evidencePath = path.join(root, "runtime-launcher", "failure-evidence.json");
  try {
    const evidence = await writePayloadFailureEvidence({
      evidencePath,
      fs,
      releaseId: "release-current",
      payloadPid: 123,
      reason: "renderer initialization failed",
      now: () => 0,
      createId: () => "test-id",
    });
    assert.equal(evidence.code, "payload_reported_error");
    assert.equal(evidence.releaseId, "release-current");
    assert.equal(evidence.details.payloadPid, "123");
    assert.deepEqual(
      JSON.parse(await fs.readFile(evidencePath, "utf8")),
      evidence,
    );
  } finally {
    await fs.rm(root, { force: true, recursive: true });
  }
});

test("payload recovery sends formatter output through the exact /self turn path", async () => {
  await withPayloadRecoveryEvidence(
    {
      code: "payload_exit_signal",
      details: { signal: "SIGTERM" },
      message: "payload terminated by signal SIGTERM",
    },
    async ({ evidencePath }) => {
      const requests = [];
      const selfThread = {
        cwd: "/workspace",
        id: "self-thread",
        name: "/self",
      };
      const result = await recoverPayloadRuntimeFailureIfPresent({
        evidencePath,
        formatPayloadRuntimeRecoveryPrompt,
        fs,
        sendSelfCommand: (text) =>
          sendSelfCommandToThread({
            appServerClient: {},
            buildTurnInput: (payload) => [{ type: "text", text: payload.text }],
            loadThreadForTurn: async () => selfThread,
            normalizeThread: (thread) => thread,
            persistSystemThreadId: async () => {},
            project: {
              id: "/self",
              managedBy: "morpheus",
              path: "/self",
              system: true,
              systemThreadId: "self-thread",
              workspace: "/workspace",
            },
            rememberThreadRuntime: () => {},
            startThreadTurn: async (payload, input) => {
              requests.push({ input, payload });
              return { turn: { id: "turn-1" } };
            },
            text,
            threads: [selfThread],
          }),
      });

      assert.deepEqual(result, {
        delivery: "sent",
        evidence: true,
        payloadEvidence: true,
        recovered: true,
        recoveryOccurrenceId: "runtime-1:release-current",
      });
      assert.equal(requests.length, 1);
      assert.match(requests[0].payload.text, /signal SIGTERM/);
      assert.equal(await fileExists(evidencePath), false);
    },
  );
});

test("payload recovery retains evidence and releases its claim when /self input fails", async () => {
  await withPayloadRecoveryEvidence({}, async ({ evidencePath }) => {
    let sent = 0;
    await assert.rejects(
      () =>
        recoverPayloadRuntimeFailureIfPresent({
          evidencePath,
          formatPayloadRuntimeRecoveryPrompt,
          fs,
          async sendSelfCommand() {
            throw new Error("turn start failed");
          },
        }),
      /turn start failed/,
    );
    assert.equal(await fileExists(evidencePath), true);
    assert.deepEqual(
      await recoverPayloadRuntimeFailureIfPresent({
        evidencePath,
        formatPayloadRuntimeRecoveryPrompt,
        fs,
        async sendSelfCommand() {
          sent += 1;
        },
      }),
      {
        delivery: "sent",
        evidence: true,
        payloadEvidence: true,
        recovered: true,
        recoveryOccurrenceId: "runtime-1:release-current",
      },
    );
    assert.equal(sent, 1);
  });
});

test("payload recovery does not send twice when evidence cleanup fails", async () => {
  await withPayloadRecoveryEvidence({}, async ({ evidencePath }) => {
    let failEvidenceRemoval = true;
    const fsWithFailedFirstCleanup = {
      ...fs,
      async rm(targetPath, options) {
        if (targetPath === evidencePath && failEvidenceRemoval) {
          failEvidenceRemoval = false;
          throw new Error("consume failed");
        }
        return fs.rm(targetPath, options);
      },
    };
    const sent = [];
    const recover = () =>
      recoverPayloadRuntimeFailureIfPresent({
        evidencePath,
        formatPayloadRuntimeRecoveryPrompt,
        fs: fsWithFailedFirstCleanup,
        async sendSelfCommand(text) {
          sent.push(text);
        },
      });

    await assert.rejects(recover, /consume failed/);
    assert.deepEqual(await recover(), {
      delivery: "already_sent",
      evidence: true,
      payloadEvidence: true,
      recovered: true,
      recoveryOccurrenceId: "runtime-1:release-current",
    });
    assert.equal(sent.length, 1);
    assert.equal(await fileExists(evidencePath), false);
  });
});

test("generic launcher evidence is not consumed by the payload adapter", async () => {
  await withPayloadRecoveryEvidence(
    {
      code: "payload_guard_blocked",
      fallbackReleaseId: undefined,
    },
    async ({ evidencePath }) => {
      const result = await recoverPayloadRuntimeFailureIfPresent({
        evidencePath,
        formatPayloadRuntimeRecoveryPrompt,
        fs,
        async sendSelfCommand() {
          throw new Error("must not send");
        },
      });
      assert.deepEqual(result, {
        evidence: true,
        payloadEvidence: false,
        recovered: false,
      });
    },
  );
});

test("startup records generic recovery before sending a payload /self prompt", async () => {
  await withPayloadRecoveryEvidence({}, async ({ evidencePath }) => {
    const requests = [];
    const sent = [];
    const result = await recoverLauncherStateAtStartup({
      recordLauncherRecovery: () =>
        recordLauncherRecoveryIfPresent({
          appServerClient: {
            async request(method, params) {
              requests.push({ method, params });
              return { recorded: true };
            },
          },
          evidencePath,
          fs,
          listThreads: async () => ({
            materializedSelfThreadId: "self",
            selfProjectThreadId: "self",
          }),
          subscribeThread: async () => {
            throw new Error("already materialized");
          },
        }),
      recoverPayloadFailure: () =>
        recoverPayloadRuntimeFailureIfPresent({
          evidencePath,
          formatPayloadRuntimeRecoveryPrompt,
          fs,
          async sendSelfCommand(text) {
            sent.push(text);
          },
        }),
    });

    assert.deepEqual(requests.map(({ method }) => method), [
      "thread/clientRecovery/record",
    ]);
    assert.equal(sent.length, 1);
    assert.equal(await fileExists(evidencePath), false);
    assert.deepEqual(result, {
      hasDurableRestartRecovery: true,
      recorded: { evidence: true, recorded: true },
      payloadRecovery: {
        delivery: "sent",
        evidence: true,
        payloadEvidence: true,
        recovered: true,
        recoveryOccurrenceId: "runtime-1:release-current",
      },
      recoveryOccurrenceId: "runtime-1:release-current",
    });

    const genericPrompts = [];
    const roots = [
      recoveryProjectRoot({
        id: "project-a",
        lifecycleStatus: { type: "active", activeFlags: ["running"] },
        updatedAt: 2,
      }),
      recoveryProjectRoot({
        id: "system-self",
        lifecycleStatus: { type: "active", activeFlags: ["running"] },
        name: "/self",
        updatedAt: 3,
      }),
      recoveryProjectRoot({
        id: "completed",
        lifecycleStatus: { type: "final", result: { type: "completed" } },
      }),
    ];
    const coordinator = createThreadAutoResumeCoordinator({
      readThread: async (threadId) => ({
        thread: roots.find((thread) => thread.id === threadId),
      }),
      subscribeThread: async () => {},
      sendResumeInput: async (thread, text) =>
        genericPrompts.push({ threadId: thread.id, text }),
      stateStore: { has: async () => false, mark: async () => {} },
      logger: { warn: () => {} },
    });
    const autoResume = await coordinator.runAfterRuntimeRestartRecovery({
      hasDurableRestartRecovery: result.hasDurableRestartRecovery,
      threads: roots,
      expectedRestart: { expectedThreadIds: [] },
    });

    assert.deepEqual(autoResume.resumedThreadIds, ["system-self", "project-a"]);
    assert.deepEqual(genericPrompts, [
      {
        threadId: "system-self",
        text: RESTART_RECOVERY_PROMPTS.projectRootFanout,
      },
      {
        threadId: "project-a",
        text: RESTART_RECOVERY_PROMPTS.projectRootFanout,
      },
    ]);
  });
});

test("startup has zero recovery sends when no evidence exists", async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "payload-cold-start-"));
  const evidencePath = path.join(root, "failure-evidence.json");
  const calls = [];
  try {
    const result = await recoverLauncherStateAtStartup({
      recordLauncherRecovery: () =>
        recordLauncherRecoveryIfPresent({
          appServerClient: {
            async request() {
              calls.push("generic-send");
            },
          },
          evidencePath,
          fs,
          listThreads: async () => {
            calls.push("list-threads");
          },
          subscribeThread: async () => {
            calls.push("subscribe");
          },
        }),
      recoverPayloadFailure: () =>
        recoverPayloadRuntimeFailureIfPresent({
          evidencePath,
          formatPayloadRuntimeRecoveryPrompt,
          fs,
          async sendSelfCommand() {
            calls.push("self-send");
          },
        }),
    });

    assert.deepEqual(calls, []);
    assert.deepEqual(result, {
      hasDurableRestartRecovery: false,
      recorded: { evidence: false, recorded: false },
      payloadRecovery: {
        evidence: false,
        payloadEvidence: false,
        recovered: false,
      },
      recoveryOccurrenceId: null,
    });
    const coordinator = createThreadAutoResumeCoordinator({
      readThread: async () => calls.push("read-thread"),
      subscribeThread: async () => calls.push("subscribe"),
      sendResumeInput: async () => calls.push("generic-send"),
      stateStore: { has: async () => false, mark: async () => {} },
      logger: { warn: () => {} },
    });
    await coordinator.runAfterRuntimeRestartRecovery({
      hasDurableRestartRecovery: result.hasDurableRestartRecovery,
      threads: [recoveryProjectRoot()],
      expectedRestart: { expectedThreadIds: [] },
    });
    assert.deepEqual(calls, []);
  } finally {
    await fs.rm(root, { force: true, recursive: true });
  }
});

test("startup still sends a payload /self prompt when generic recovery is complete", async () => {
  const calls = [];
  const result = await recoverLauncherStateAtStartup({
    logger: {
      error() {
        calls.push("payload-error");
      },
    },
    async recoverPayloadFailure() {
      calls.push("payload-self");
      const error = new Error("self turn unavailable");
      error.payloadEvidence = true;
      throw error;
    },
    async recordLauncherRecovery() {
      calls.push("generic-fanout");
      return { recorded: true };
    },
  });

  assert.deepEqual(calls, [
    "generic-fanout",
    "payload-self",
    "payload-error",
  ]);
  assert.deepEqual(result, {
    hasDurableRestartRecovery: true,
    payloadRecovery: null,
    recorded: { recorded: true },
    recoveryOccurrenceId: null,
  });
});

test("concurrent payload recovery leaves an in-flight claim alone", async () => {
  await withPayloadRecoveryEvidence({}, async ({ evidencePath }) => {
    let releaseSend;
    const waitForSendRelease = new Promise((resolve) => {
      releaseSend = resolve;
    });
    let notifySendStarted;
    const sendStarted = new Promise((resolve) => {
      notifySendStarted = resolve;
    });
    let sent = 0;
    const first = recoverPayloadRuntimeFailureIfPresent({
      evidencePath,
      formatPayloadRuntimeRecoveryPrompt,
      fs,
      async sendSelfCommand() {
        sent += 1;
        notifySendStarted();
        await waitForSendRelease;
      },
    });
    await sendStarted;
    const second = await recoverPayloadRuntimeFailureIfPresent({
      evidencePath,
      formatPayloadRuntimeRecoveryPrompt,
      fs,
      async sendSelfCommand() {
        sent += 1;
      },
    });

    assert.deepEqual(second, {
      delivery: "in_flight",
      evidence: true,
      payloadEvidence: true,
      recovered: false,
      recoveryOccurrenceId: "runtime-1:release-current",
    });
    assert.equal(await fileExists(evidencePath), true);
    releaseSend();
    await first;
    assert.equal(sent, 1);
    assert.equal(await fileExists(evidencePath), false);
  });
});

test("payload cleanup does not remove a newer failure event", async () => {
  await withPayloadRecoveryEvidence({}, async ({ evidencePath }) => {
    let releaseSend;
    const waitForSendRelease = new Promise((resolve) => {
      releaseSend = resolve;
    });
    let notifySendStarted;
    const sendStarted = new Promise((resolve) => {
      notifySendStarted = resolve;
    });
    const first = recoverPayloadRuntimeFailureIfPresent({
      evidencePath,
      formatPayloadRuntimeRecoveryPrompt,
      fs,
      async sendSelfCommand() {
        notifySendStarted();
        await waitForSendRelease;
      },
    });
    await sendStarted;
    const newer = {
      activationId: "runtime-2",
      releaseId: "release-newer",
      occurredAt: "2026-09-10T00:00:01.000Z",
      fallbackReleaseId: "release-seed",
      code: "payload_reported_error",
      message: "newer failure",
      details: { payloadPid: "2", source: "payload" },
    };
    await fs.writeFile(evidencePath, `${JSON.stringify(newer)}\n`);
    releaseSend();
    await first;

    assert.deepEqual(
      JSON.parse(await fs.readFile(evidencePath, "utf8")),
      newer,
    );
  });
});

async function withPayloadRecoveryEvidence(overrides, run) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "payload-recovery-"));
  const evidencePath = path.join(root, "failure-evidence.json");
  const evidence = {
    activationId: "runtime-1",
    releaseId: "release-current",
    occurredAt: "2026-09-10T00:00:00.000Z",
    fallbackReleaseId: "release-seed",
    code: "payload_exit_code",
    message: "payload exited with code 9",
    details: { exitCode: "9" },
    ...overrides,
  };
  try {
    await fs.writeFile(evidencePath, `${JSON.stringify(evidence)}\n`);
    await run({ evidencePath });
  } finally {
    await fs.rm(root, { force: true, recursive: true });
  }
}

async function fileExists(targetPath) {
  try {
    await fs.access(targetPath);
    return true;
  } catch (error) {
    if (error?.code === "ENOENT") {
      return false;
    }
    throw error;
  }
}

function recoveryProjectRoot(overrides = {}) {
  return {
    cwd: "/workspace/project",
    id: "project-root",
    lifecycleStatus: { type: "final", result: { type: "interrupted" } },
    model: "gpt",
    modelProvider: "openai",
    reasoningEffort: "medium",
    source: "appServer",
    threadSource: "user",
    turns: [],
    updatedAt: 1,
    ...overrides,
  };
}

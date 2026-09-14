const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");

const {
  createRuntimeRestartController,
  createRuntimeRestartIntentStore,
  MAX_REQUEST_ID_BYTES,
  recoverRuntimeRestartAfterThreadTerminal,
} = require("./runtimeRestartIntent.cjs");
const {
  expectedRuntimeRestartRecoveryPrompt,
} = require("./restartRecoveryPrompts.cjs");
const {
  notifyRecoverableRestartErrorOnSelf,
} = require("./restartRecoverySelfNotice.cjs");

async function withStore(run, initialState = null) {
  const directory = await fs.mkdtemp(
    path.join(os.tmpdir(), "runtime-restart-intent-test-"),
  );
  try {
    const filePath = path.join(directory, "intents.json");
    if (initialState) {
      await fs.writeFile(filePath, JSON.stringify(initialState), "utf8");
    }
    const store = createRuntimeRestartIntentStore(
      filePath,
      { fs },
    );
    await run(store);
  } finally {
    await fs.rm(directory, { recursive: true, force: true });
  }
}

function notification(requestId, threadId = "thread-1") {
  return {
    method: "client/relaunch/requested",
    params: {
      requestId,
      requestedByThreadId: threadId,
      reason: "runtime update",
    },
  };
}

test("controller persists received intent before executing restart", async () => {
  await withStore(async (store) => {
    let releaseExecution;
    const execution = new Promise((resolve) => {
      releaseExecution = resolve;
    });
    let markExecutionStarted;
    const executionStarted = new Promise((resolve) => {
      markExecutionStarted = resolve;
    });
    let executingRecord;
    const controller = createRuntimeRestartController({
      store,
      execute: async () => {
        [executingRecord] = await store.recoverable();
        markExecutionStarted();
        return execution;
      },
      recover: async () => {
        throw new Error("completed restart must recover after Host restart");
      },
      logger: { error: () => {}, warn: () => {} },
    });

    const accepted = await controller.handle(notification("restart-1"));
    assert.equal(accepted.ok, true);
    assert.equal(accepted.persisted, true);
    assert.equal(accepted.record.phase, "received");
    await executionStarted;
    assert.equal(executingRecord.requestId, "restart-1");
    assert.equal(executingRecord.phase, "executing");

    releaseExecution({ ok: true });
    await controller.waitForIdle();
    assert.equal((await store.recoverable())[0].phase, "completed");
  });
});

test("controller broadcasts runtime restart progress snapshots", async () => {
  await withStore(async (store) => {
    const statuses = [];
    let releaseExecution;
    const execution = new Promise((resolve) => {
      releaseExecution = resolve;
    });
    let markExecutionStarted;
    const executionStarted = new Promise((resolve) => {
      markExecutionStarted = resolve;
    });
    const controller = createRuntimeRestartController({
      store,
      execute: async () => {
        markExecutionStarted();
        return execution;
      },
      recover: async () => {
        throw new Error("completed restart must recover after Host restart");
      },
      broadcastStatus: (status) => statuses.push(status),
      logger: { error: () => {}, warn: () => {} },
    });

    await controller.handle(notification("restart-1"));
    await executionStarted;
    releaseExecution({ ok: true });
    await controller.waitForIdle();

    assert.deepEqual(
      statuses.map((status) => status.runtimeRestart?.phase),
      ["received", "executing", "completed"],
    );
    assert.deepEqual(
      statuses.map((status) => status.runtimeRestart?.requestId),
      ["restart-1", "restart-1", "restart-1"],
    );
    assert.deepEqual(
      statuses.map((status) => status.runtimeRestart?.requestedByThreadId),
      ["thread-1", "thread-1", "thread-1"],
    );
  });
});

test("controller can persist completed handoff before expected Host exit", async () => {
  await withStore(async (store) => {
    let markHandoffPersisted;
    const handoffPersisted = new Promise((resolve) => {
      markHandoffPersisted = resolve;
    });
    const controller = createRuntimeRestartController({
      store,
      execute: async (_notification, handoff) => {
        await handoff.markExpectedRestartHandoffReady();
        const [completed] = await store.recoverable();
        assert.equal(completed.requestId, "restart-1");
        assert.equal(completed.phase, "completed");
        assert.equal(completed.completedByHostInstanceId, "host-1");
        markHandoffPersisted();
        return new Promise(() => {});
      },
      recover: async () => {
        throw new Error("current Host must not recover its own completed handoff");
      },
      hostInstanceId: "host-1",
      logger: { error: () => {}, warn: () => {} },
    });

    const accepted = await controller.handle(notification("restart-1"));
    assert.equal(accepted.ok, true);
    await handoffPersisted;

    const sameHostRecovery = await controller.recoverPending();
    assert.deepEqual(sameHostRecovery.recoveredThreadIds, []);
    assert.deepEqual(sameHostRecovery.failedThreadIds, []);
    assert.deepEqual(sameHostRecovery.expectedRequestIds, ["restart-1"]);
    assert.deepEqual(sameHostRecovery.expectedThreadIds, ["thread-1"]);
    assert.equal(
      sameHostRecovery.recoveryOccurrenceId,
      "runtime-restart:restart-1",
    );

    const recovered = [];
    const nextHost = createRuntimeRestartController({
      store,
      execute: async () => {
        throw new Error("recovery must not execute another restart");
      },
      recover: async (record) => recovered.push(record),
      hostInstanceId: "host-2",
      logger: { error: () => {}, warn: () => {} },
    });
    const nextHostRecovery = await nextHost.recoverPending();

    assert.deepEqual(nextHostRecovery.recoveredThreadIds, ["thread-1"]);
    assert.deepEqual(nextHostRecovery.expectedRequestIds, ["restart-1"]);
    assert.equal(
      nextHostRecovery.recoveryOccurrenceId,
      "runtime-restart:restart-1",
    );
    assert.equal(recovered[0].phase, "completed");
    assert.equal(recovered[0].completedByHostInstanceId, "host-1");
    assert.deepEqual(await store.recoverable(), []);
  });
});

test("concurrent restart requests coalesce into one Runtime Capsule restart", async () => {
  await withStore(async (store) => {
    let releaseExecution;
    const execution = new Promise((resolve) => {
      releaseExecution = resolve;
    });
    let executions = 0;
    const controller = createRuntimeRestartController({
      store,
      execute: async () => {
        executions += 1;
        return execution;
      },
      recover: async () => {
        throw new Error("completed restart must recover after Host restart");
      },
      logger: { error: () => {}, warn: () => {} },
    });

    assert.equal((await controller.handle(notification("one"))).ok, true);
    assert.equal(
      (await controller.handle(notification("two"))).coalesced,
      true,
    );
    assert.equal(executions, 1);
    assert.equal(
      (await store.recoverable()).find(
        (record) => record.requestId === "one",
      ).phase,
      "executing",
    );

    releaseExecution({ ok: true });
    await controller.waitForIdle();
    assert.equal(executions, 1);
    assert.deepEqual(
      (await store.recoverable())
        .map((record) => [record.requestId, record.phase])
        .sort(),
      [
        ["one", "completed"],
        ["two", "completed"],
      ],
    );

    const recovered = [];
    const recoveryController = createRuntimeRestartController({
      store,
      execute: async () => {
        throw new Error("recovery must not execute a restart");
      },
      recover: async (record) => recovered.push(record),
      logger: { error: () => {}, warn: () => {} },
    });
    await recoveryController.recoverPending();
    assert.deepEqual(
      recovered.map((record) => record.requestId).sort(),
      ["one"],
    );
    assert.deepEqual(await store.recoverable(), []);
  });
});

test("restart requests with obsolete mode are rejected before persistence", async () => {
  await withStore(async (store) => {
    for (const mode of [undefined, null, "full", "hot"]) {
      const obsolete = notification(`obsolete-mode-${String(mode)}`);
      obsolete.params.mode = mode;

      const result = await store.accept(obsolete);

      assert.equal(result.kind, "invalid");
      assert.match(result.reason, /do not support mode/);
    }
    assert.deepEqual(await store.recoverable(), []);
  });
});

test("completed restart is recoverable only from a new Host instance", async () => {
  await withStore(async (store) => {
    let releaseExecution;
    const execution = new Promise((resolve) => {
      releaseExecution = resolve;
    });
    let markExecutionStarted;
    const executionStarted = new Promise((resolve) => {
      markExecutionStarted = resolve;
    });
    let currentHostRecoveries = 0;
    const currentHost = createRuntimeRestartController({
      store,
      execute: async () => {
        markExecutionStarted();
        return execution;
      },
      recover: async () => {
        currentHostRecoveries += 1;
      },
      hostInstanceId: "host-1",
      logger: { error: () => {}, warn: () => {} },
    });
    await currentHost.handle(notification("restart-1"));
    await executionStarted;

    const earlyTerminalRecovery =
      await recoverRuntimeRestartAfterThreadTerminal(currentHost, {
        method: "thread/status/changed",
        params: {
          threadId: "thread-1",
          lifecycleStatus: {
            type: "final",
            result: { type: "completed" },
          },
        },
      });
    assert.deepEqual(earlyTerminalRecovery.recoveredThreadIds, []);
    assert.equal(currentHostRecoveries, 0);

    releaseExecution({ ok: true });
    await currentHost.waitForIdle();
    assert.equal(currentHostRecoveries, 0);

    const sameHostRecovery = await currentHost.recoverPending();
    assert.deepEqual(sameHostRecovery.recoveredThreadIds, []);
    assert.deepEqual(sameHostRecovery.failedThreadIds, []);
    assert.deepEqual(sameHostRecovery.expectedRequestIds, ["restart-1"]);
    assert.deepEqual(sameHostRecovery.expectedThreadIds, ["thread-1"]);
    assert.equal(
      sameHostRecovery.recoveryOccurrenceId,
      "runtime-restart:restart-1",
    );

    const recovered = [];
    const nextHost = createRuntimeRestartController({
      store,
      execute: async () => {
        throw new Error("recovery must not execute another restart");
      },
      recover: async (record) => recovered.push(record),
      hostInstanceId: "host-2",
      logger: { error: () => {}, warn: () => {} },
    });
    const nextHostRecovery = await nextHost.recoverPending();

    assert.deepEqual(nextHostRecovery.recoveredThreadIds, ["thread-1"]);
    assert.deepEqual(nextHostRecovery.expectedRequestIds, ["restart-1"]);
    assert.equal(
      nextHostRecovery.recoveryOccurrenceId,
      "runtime-restart:restart-1",
    );
    assert.equal(recovered[0].completedByHostInstanceId, "host-1");
    assert.deepEqual(await store.recoverable(), []);
  });
});

test("completed restart recovery consumes only after durable /self notice recording", async () => {
  await withStore(
    async (store) => {
      const selfThread = {
        id: "self-thread",
        name: "/self",
        turns: [],
      };
      const calls = [];
      const controller = createRuntimeRestartController({
        store,
        execute: async () => {
          throw new Error("recovery must not execute another restart");
        },
        recover: (record) =>
          notifyRecoverableRestartErrorOnSelf({
            sourceThreadId: record.requestedByThreadId,
            noticeId: `runtime-restart-recovery:${record.requestId}`,
            prompt: expectedRuntimeRestartRecoveryPrompt(record),
            listThreads: async () => {
              throw new Error("source /self should be used directly");
            },
            readThread: async (threadId) => {
              calls.push(["read", threadId]);
              return { thread: selfThread };
            },
            subscribeThread: async (threadId) => {
              calls.push(["subscribe", threadId]);
            },
            submitRecoveryMessage: async (thread, message) => {
              calls.push(["submit", thread.id, message.id, message.text]);
              thread.turns.push({
                items: [
                  {
                    type: "userMessage",
                    id: message.id,
                    content: [{ type: "text", text: message.text }],
                  },
                ],
              });
              return {};
            },
          }),
        hostInstanceId: "host-new",
        logger: { error: () => {}, warn: () => {} },
      });

      const result = await controller.recoverPending();

      assert.deepEqual(result.recoveredThreadIds, ["self-thread"]);
      assert.deepEqual(result.failedThreadIds, []);
      assert.deepEqual(await store.recoverable(), []);
      assert.deepEqual(calls.map((call) => call.slice(0, 3)), [
        ["read", "self-thread"],
        ["subscribe", "self-thread"],
        ["submit", "self-thread", "runtime-restart-recovery:restart-1"],
        ["read", "self-thread"],
      ]);
      assert.match(
        selfThread.turns[0].items[0].content[0].text,
        /Morpheus 已恢复预期的 Runtime Capsule 重启请求 restart-1/,
      );

      const duplicate = await store.accept(notification("restart-1", "self-thread"));
      assert.equal(duplicate.kind, "duplicate");
      assert.equal(duplicate.record.phase, "consumed");
      assert.equal(duplicate.record.outcomePhase, "completed");
    },
    {
      version: 2,
      records: [
        {
          requestId: "restart-1",
          requestedByThreadId: "self-thread",
          reason: "runtime update",
          phase: "completed",
          completedByHostInstanceId: "host-old",
          createdAtMs: 1,
          updatedAtMs: 2,
        },
      ],
    },
  );
});

test("completed restart recovery releases claim when submitted notice is not durable", async () => {
  await withStore(
    async (store) => {
      const controller = createRuntimeRestartController({
        store,
        execute: async () => {
          throw new Error("recovery must not execute another restart");
        },
        recover: (record) =>
          notifyRecoverableRestartErrorOnSelf({
            sourceThreadId: record.requestedByThreadId,
            noticeId: `runtime-restart-recovery:${record.requestId}`,
            prompt: expectedRuntimeRestartRecoveryPrompt(record),
            listThreads: async () => {
              throw new Error("source /self should be used directly");
            },
            readThread: async (threadId) => ({
              thread: { id: threadId, name: "/self", turns: [] },
            }),
            subscribeThread: async () => {},
            submitRecoveryMessage: async () => ({}),
            verifyAttempts: 1,
          }),
        hostInstanceId: "host-new",
        logger: { error: () => {}, warn: () => {} },
      });

      const result = await controller.recoverPending();
      const [record] = await store.recoverable();

      assert.deepEqual(result.recoveredThreadIds, []);
      assert.deepEqual(result.failedThreadIds, ["self-thread"]);
      assert.equal(record.requestId, "restart-1");
      assert.equal(record.phase, "completed");

      const duplicate = await store.accept(notification("restart-1", "self-thread"));
      assert.notEqual(duplicate.record.phase, "consumed");
    },
    {
      version: 2,
      records: [
        {
          requestId: "restart-1",
          requestedByThreadId: "self-thread",
          reason: "runtime update",
          phase: "completed",
          completedByHostInstanceId: "host-old",
          createdAtMs: 1,
          updatedAtMs: 2,
        },
      ],
    },
  );
});

test("completed restart recovery retry consumes an already-submitted user notice", async () => {
  await withStore(
    async (store) => {
      let readCount = 0;
      let submitCount = 0;
      let submittedItem = null;
      const controller = createRuntimeRestartController({
        store,
        execute: async () => {
          throw new Error("recovery must not execute another restart");
        },
        recover: (record) =>
          notifyRecoverableRestartErrorOnSelf({
            sourceThreadId: record.requestedByThreadId,
            noticeId: `runtime-restart-recovery:${record.requestId}`,
            prompt: expectedRuntimeRestartRecoveryPrompt(record),
            listThreads: async () => {
              throw new Error("source /self should be used directly");
            },
            readThread: async (threadId) => {
              readCount += 1;
              return {
                thread: {
                  id: threadId,
                  name: "/self",
                  turns:
                    submittedItem && readCount >= 3
                      ? [{ items: [submittedItem] }]
                      : [],
                },
              };
            },
            subscribeThread: async () => {},
            submitRecoveryMessage: async (_thread, message) => {
              submitCount += 1;
              submittedItem = {
                type: "userMessage",
                id: message.id,
                content: [{ type: "text", text: message.text }],
              };
              return { submitted: true };
            },
            verifyAttempts: 1,
          }),
        hostInstanceId: "host-new",
        logger: { error: () => {}, warn: () => {} },
      });

      const first = await controller.recoverPending();
      const second = await controller.recoverPending();

      assert.deepEqual(first.recoveredThreadIds, []);
      assert.deepEqual(first.failedThreadIds, ["self-thread"]);
      assert.deepEqual(second.recoveredThreadIds, ["self-thread"]);
      assert.deepEqual(second.failedThreadIds, []);
      assert.equal(submitCount, 1);
      assert.deepEqual(await store.recoverable(), []);
    },
    {
      version: 2,
      records: [
        {
          requestId: "restart-1",
          requestedByThreadId: "self-thread",
          reason: "runtime update",
          phase: "completed",
          completedByHostInstanceId: "host-old",
          createdAtMs: 1,
          updatedAtMs: 2,
        },
      ],
    },
  );
});

test("completed restart recovery ignores existing typed client recovery until user message is durable", async () => {
  await withStore(
    async (store) => {
      let submitted = false;
      const calls = [];
      const controller = createRuntimeRestartController({
        store,
        execute: async () => {
          throw new Error("recovery must not execute another restart");
        },
        recover: (record) =>
          notifyRecoverableRestartErrorOnSelf({
            sourceThreadId: record.requestedByThreadId,
            noticeId: `runtime-restart-recovery:${record.requestId}`,
            prompt: expectedRuntimeRestartRecoveryPrompt(record),
            readThread: async (threadId) => ({
              thread: {
                id: threadId,
                name: "/self",
                turns: [
                  {
                    items: [
                      {
                        type: "clientRecovery",
                        id: `runtime-restart-recovery:${record.requestId}`,
                        reason: expectedRuntimeRestartRecoveryPrompt(record),
                      },
                      ...(submitted
                        ? [
                            {
                              type: "userMessage",
                              id: "submitted-user-message",
                              content: [
                                {
                                  type: "text",
                                  text: expectedRuntimeRestartRecoveryPrompt(record),
                                },
                              ],
                            },
                          ]
                        : []),
                    ],
                  },
                ],
              },
            }),
            subscribeThread: async (threadId) => calls.push(["subscribe", threadId]),
            submitRecoveryMessage: async (thread, message) => {
              calls.push(["submit", thread.id, message.id]);
              submitted = true;
              return {};
            },
          }),
        hostInstanceId: "host-new",
        logger: { error: () => {}, warn: () => {} },
      });

      const result = await controller.recoverPending();

      assert.deepEqual(result.recoveredThreadIds, ["self-thread"]);
      assert.deepEqual(result.failedThreadIds, []);
      assert.deepEqual(calls, [
        ["subscribe", "self-thread"],
        ["submit", "self-thread", "runtime-restart-recovery:restart-1"],
      ]);
      assert.deepEqual(await store.recoverable(), []);
    },
    {
      version: 2,
      records: [
        {
          requestId: "restart-1",
          requestedByThreadId: "self-thread",
          reason: "runtime update",
          phase: "completed",
          completedByHostInstanceId: "host-old",
          createdAtMs: 1,
          updatedAtMs: 2,
        },
      ],
    },
  );
});

test("completed restart recovery releases claim when durable notice submission fails", async () => {
  await withStore(
    async (store) => {
      const controller = createRuntimeRestartController({
        store,
        execute: async () => {
          throw new Error("recovery must not execute another restart");
        },
        recover: (record) =>
          notifyRecoverableRestartErrorOnSelf({
            sourceThreadId: record.requestedByThreadId,
            noticeId: `runtime-restart-recovery:${record.requestId}`,
            prompt: expectedRuntimeRestartRecoveryPrompt(record),
            listThreads: async () => {
              throw new Error("source /self should be used directly");
            },
            readThread: async (threadId) => ({
              thread: { id: threadId, name: "/self", turns: [] },
            }),
            subscribeThread: async () => {},
            submitRecoveryMessage: async () => {
              throw new Error("durable submission unavailable");
            },
          }),
        hostInstanceId: "host-new",
        logger: { error: () => {}, warn: () => {} },
      });

      const result = await controller.recoverPending();
      const [record] = await store.recoverable();

      assert.deepEqual(result.recoveredThreadIds, []);
      assert.deepEqual(result.failedThreadIds, ["self-thread"]);
      assert.equal(record.requestId, "restart-1");
      assert.equal(record.phase, "completed");
      assert.equal(record.completedByHostInstanceId, "host-old");
    },
    {
      version: 2,
      records: [
        {
          requestId: "restart-1",
          requestedByThreadId: "self-thread",
          reason: "runtime update",
          phase: "completed",
          completedByHostInstanceId: "host-old",
          createdAtMs: 1,
          updatedAtMs: 2,
        },
      ],
    },
  );
});

test("completed restart recovery releases claim when source /self read mismatches", async () => {
  await withStore(
    async (store) => {
      const controller = createRuntimeRestartController({
        store,
        execute: async () => {
          throw new Error("recovery must not execute another restart");
        },
        recover: (record) =>
          notifyRecoverableRestartErrorOnSelf({
            sourceThreadId: record.requestedByThreadId,
            noticeId: `runtime-restart-recovery:${record.requestId}`,
            prompt: expectedRuntimeRestartRecoveryPrompt(record),
            listThreads: async () => ({ selfProjectThreadId: "self-thread" }),
            readThread: async (threadId) => ({
              thread:
                threadId === "self-thread"
                  ? { id: "another-thread", name: "/self", turns: [] }
                  : { id: threadId, name: "/self", turns: [] },
            }),
            subscribeThread: async () => {
              throw new Error("must not subscribe");
            },
            submitRecoveryMessage: async () => {
              throw new Error("must not submit");
            },
          }),
        hostInstanceId: "host-new",
        logger: { error: () => {}, warn: () => {} },
      });

      const result = await controller.recoverPending();
      const [record] = await store.recoverable();

      assert.deepEqual(result.recoveredThreadIds, []);
      assert.deepEqual(result.failedThreadIds, ["self-thread"]);
      assert.equal(record.requestId, "restart-1");
      assert.equal(record.phase, "completed");
    },
    {
      version: 2,
      records: [
        {
          requestId: "restart-1",
          requestedByThreadId: "self-thread",
          reason: "runtime update",
          phase: "completed",
          completedByHostInstanceId: "host-old",
          createdAtMs: 1,
          updatedAtMs: 2,
        },
      ],
    },
  );
});

test("completed restart recovery consumes when the durable notice already exists", async () => {
  await withStore(
    async (store) => {
      const controller = createRuntimeRestartController({
        store,
        execute: async () => {
          throw new Error("recovery must not execute another restart");
        },
        recover: (record) =>
          notifyRecoverableRestartErrorOnSelf({
            sourceThreadId: record.requestedByThreadId,
            noticeId: `runtime-restart-recovery:${record.requestId}`,
            prompt: expectedRuntimeRestartRecoveryPrompt(record),
            listThreads: async () => {
              throw new Error("source /self should be used directly");
            },
            readThread: async (threadId) => ({
              thread: {
                id: threadId,
                name: "/self",
                turns: [
                  {
                    items: [
                      {
                        type: "userMessage",
                        id: "runtime-restart-recovery:restart-1",
                        content: [
                          {
                            type: "text",
                            text: expectedRuntimeRestartRecoveryPrompt(record),
                          },
                        ],
                      },
                    ],
                  },
                ],
              },
            }),
            subscribeThread: async () => {
              throw new Error("must not subscribe");
            },
            submitRecoveryMessage: async () => {
              throw new Error("must not submit");
            },
          }),
        hostInstanceId: "host-new",
        logger: { error: () => {}, warn: () => {} },
      });

      const result = await controller.recoverPending();

      assert.deepEqual(result.recoveredThreadIds, ["self-thread"]);
      assert.deepEqual(await store.recoverable(), []);
      const duplicate = await store.accept(notification("restart-1", "self-thread"));
      assert.equal(duplicate.record.phase, "consumed");
    },
    {
      version: 2,
      records: [
        {
          requestId: "restart-1",
          requestedByThreadId: "self-thread",
          reason: "runtime update",
          phase: "completed",
          completedByHostInstanceId: "host-old",
          createdAtMs: 1,
          updatedAtMs: 2,
        },
      ],
    },
  );
});

test("persisted obsolete mode intents are ignored", async () => {
  await withStore(
    async (store) => {
      assert.deepEqual(await store.recoverable(), []);
    },
    {
      version: 1,
      records: [
        {
          requestId: "obsolete-mode",
          requestedByThreadId: "thread-1",
          mode: "full",
          phase: "completed",
        },
      ],
    },
  );
});

test("duplicate request id does not execute twice across controller instances", async () => {
  await withStore(async (store) => {
    let executions = 0;
    const first = createRuntimeRestartController({
      store,
      execute: async () => {
        executions += 1;
        return { ok: true };
      },
      recover: async () => {
        throw new Error("completed restart must recover after Host restart");
      },
      logger: { error: () => {}, warn: () => {} },
    });
    await first.handle(notification("restart-1"));
    await first.waitForIdle();

    const second = createRuntimeRestartController({
      store,
      execute: async () => {
        executions += 1;
        return { ok: true };
      },
      recover: async () => {},
      logger: { error: () => {}, warn: () => {} },
    });
    const duplicate = await second.handle(notification("restart-1"));

    assert.equal(duplicate.duplicate, true);
    assert.equal(executions, 1);
  });
});

test("failed execution is durable and recovery prompt prevents duplicate same-request restart", async () => {
  await withStore(async (store) => {
    const recovered = [];
    let releaseExecution;
    const execution = new Promise((resolve) => {
      releaseExecution = resolve;
    });
    let markExecutionStarted;
    const executionStarted = new Promise((resolve) => {
      markExecutionStarted = resolve;
    });
    const controller = createRuntimeRestartController({
      store,
      execute: async () => {
        markExecutionStarted();
        return execution;
      },
      recover: async (pending) => recovered.push(pending),
      logger: { error: () => {}, warn: () => {} },
    });
    await controller.handle(notification("restart-1"));
    await executionStarted;

    const earlyTerminalRecovery =
      await recoverRuntimeRestartAfterThreadTerminal(controller, {
        method: "thread/status/changed",
        params: {
          threadId: "thread-1",
          lifecycleStatus: {
            type: "final",
            result: { type: "completed" },
          },
        },
      });
    assert.deepEqual(earlyTerminalRecovery.recoveredThreadIds, []);
    assert.equal(recovered.length, 0);
    assert.equal((await store.recoverable())[0].phase, "executing");

    releaseExecution({ ok: false, reason: "build failed" });
    await controller.waitForIdle();

    const [record] = recovered;
    assert.equal(record.phase, "failed");
    assert.equal(record.error, "build failed");
    assert.equal(recovered.length, 1);
    const prompt = expectedRuntimeRestartRecoveryPrompt(record);
    assert.match(prompt, /预期的 Runtime Capsule 重启请求/);
    assert.match(prompt, /请勿为了同一个请求连续调用/);
    assert.match(prompt, /新的代码修改/);
    assert.doesNotMatch(prompt, /\(hot\)|\(full\)/);
    assert.equal(recovered[0].requestId, "restart-1");
    assert.deepEqual(await store.recoverable(), []);
  });
});

test("an old terminal fact does not recover a newer failed restart generation", async () => {
  await withStore(async (store) => {
    const recovered = [];
    const controller = createRuntimeRestartController({
      store,
      execute: async () => ({ ok: false, reason: "build failed" }),
      recover: async (record) => recovered.push(record),
      logger: { error: () => {}, warn: () => {} },
    });
    const finalNotification = {
      method: "thread/status/changed",
      params: {
        threadId: "thread-1",
        lifecycleStatus: {
          type: "final",
          result: { type: "completed" },
        },
      },
    };

    await recoverRuntimeRestartAfterThreadTerminal(
      controller,
      finalNotification,
    );
    await controller.handle(notification("restart-1"));
    await controller.waitForIdle();

    assert.equal(recovered.length, 0);
    assert.equal((await store.recoverable())[0].phase, "failed");

    await recoverRuntimeRestartAfterThreadTerminal(
      controller,
      finalNotification,
    );
    assert.equal(recovered.length, 1);
    assert.deepEqual(await store.recoverable(), []);
  });
});

test("coalesced failed restarts recover every terminal originating thread", async () => {
  await withStore(async (store) => {
    let releaseExecution;
    const execution = new Promise((resolve) => {
      releaseExecution = resolve;
    });
    let markExecutionStarted;
    const executionStarted = new Promise((resolve) => {
      markExecutionStarted = resolve;
    });
    const recovered = [];
    const controller = createRuntimeRestartController({
      store,
      execute: async () => {
        markExecutionStarted();
        return execution;
      },
      recover: async (record) => recovered.push(record),
      logger: { error: () => {}, warn: () => {} },
    });

    await controller.handle(notification("restart-a", "thread-a"));
    await executionStarted;
    const coalesced = await controller.handle(
      notification("restart-b", "thread-b"),
    );
    assert.equal(coalesced.coalesced, true);

    for (const threadId of ["thread-a", "thread-b"]) {
      await recoverRuntimeRestartAfterThreadTerminal(controller, {
        method: "thread/status/changed",
        params: {
          threadId,
          lifecycleStatus: {
            type: "final",
            result: { type: "completed" },
          },
        },
      });
    }
    assert.equal(recovered.length, 0);

    releaseExecution({ ok: false, reason: "build failed" });
    await controller.waitForIdle();

    assert.deepEqual(
      recovered.map((record) => record.requestId).sort(),
      ["restart-a", "restart-b"],
    );
    assert.deepEqual(await store.recoverable(), []);
  });
});

test("same-thread coalesced failed restarts create one recovery turn", async () => {
  await withStore(async (store) => {
    let releaseExecution;
    const execution = new Promise((resolve) => {
      releaseExecution = resolve;
    });
    let markExecutionStarted;
    const executionStarted = new Promise((resolve) => {
      markExecutionStarted = resolve;
    });
    const recovered = [];
    const controller = createRuntimeRestartController({
      store,
      execute: async () => {
        markExecutionStarted();
        return execution;
      },
      recover: async (record) => recovered.push(record),
      logger: { error: () => {}, warn: () => {} },
    });

    await controller.handle(notification("restart-a"));
    await executionStarted;
    assert.equal(
      (await controller.handle(notification("restart-b"))).coalesced,
      true,
    );
    await recoverRuntimeRestartAfterThreadTerminal(controller, {
      method: "thread/status/changed",
      params: {
        threadId: "thread-1",
        lifecycleStatus: {
          type: "final",
          result: { type: "completed" },
        },
      },
    });

    releaseExecution({ ok: false, reason: "build failed" });
    await controller.waitForIdle();

    assert.deepEqual(
      recovered.map((record) => record.requestId),
      ["restart-a"],
    );
    assert.deepEqual(await store.recoverable(), []);
  });
});

test("atomic persistence failure prevents restart execution", async () => {
  const directory = await fs.mkdtemp(
    path.join(os.tmpdir(), "runtime-restart-intent-test-"),
  );
  try {
    const failingFs = {
      ...fs,
      rename: async () => {
        const error = new Error("atomic rename failed");
        error.code = "EIO";
        throw error;
      },
    };
    const store = createRuntimeRestartIntentStore(
      path.join(directory, "intents.json"),
      { fs: failingFs },
    );
    let executions = 0;
    const controller = createRuntimeRestartController({
      store,
      execute: async () => {
        executions += 1;
        return { ok: true };
      },
      recover: async () => {},
      logger: { error: () => {}, warn: () => {} },
    });

    const result = await controller.handle(notification("restart-1"));

    assert.equal(result.ok, false);
    assert.equal(result.persisted, false);
    assert.match(result.reason, /atomic rename failed/);
    assert.equal(executions, 0);
  } finally {
    await fs.rm(directory, { recursive: true, force: true });
  }
});

test("concurrent recoverPending calls claim each intent once", async () => {
  await withStore(async (store) => {
    await store.accept(notification("restart-1"));
    await store.updateGroup("restart-1", "completed");
    let recoveries = 0;
    let releaseRecovery;
    const recoveryBlocked = new Promise((resolve) => {
      releaseRecovery = resolve;
    });
    const controller = createRuntimeRestartController({
      store,
      execute: async () => {
        throw new Error("recovery must not execute a restart");
      },
      recover: async () => {
        recoveries += 1;
        await recoveryBlocked;
      },
      logger: { error: () => {}, warn: () => {} },
    });

    const first = controller.recoverPending();
    const second = controller.recoverPending();
    assert.equal(first, second);
    releaseRecovery();
    const [firstResult, secondResult] = await Promise.all([first, second]);

    assert.equal(recoveries, 1);
    assert.deepEqual(firstResult, secondResult);
    assert.deepEqual(await store.recoverable(), []);
  });
});

test("failed recovery releases its durable claim for a later retry", async () => {
  await withStore(async (store) => {
    await store.accept(notification("restart-1"));
    await store.updateGroup("restart-1", "failed", "build failed");
    const first = createRuntimeRestartController({
      store,
      execute: async () => {
        throw new Error("recovery must not execute a restart");
      },
      recover: async () => {
        throw new Error("thread unavailable");
      },
      logger: { error: () => {}, warn: () => {} },
    });

    const failed = await first.recoverPending();
    assert.deepEqual(failed.failedThreadIds, ["thread-1"]);
    assert.equal((await store.recoverable())[0].phase, "failed");

    let recoveries = 0;
    const second = createRuntimeRestartController({
      store,
      execute: async () => {
        throw new Error("recovery must not execute a restart");
      },
      recover: async () => {
        recoveries += 1;
      },
      logger: { error: () => {}, warn: () => {} },
    });
    const retried = await second.recoverPending();

    assert.equal(recoveries, 1);
    assert.deepEqual(retried.recoveredThreadIds, ["thread-1"]);
    assert.deepEqual(retried.expectedRequestIds, ["restart-1"]);
    assert.equal(retried.recoveryOccurrenceId, "runtime-restart:restart-1");
    assert.deepEqual(await store.recoverable(), []);
  });
});

test("execution outcome update preserves an in-progress recovery claim", async () => {
  await withStore(async (store) => {
    await store.accept(notification("restart-1"));
    await store.updateGroup("restart-1", "executing");
    let markRecoveryStarted;
    const recoveryStarted = new Promise((resolve) => {
      markRecoveryStarted = resolve;
    });
    let releaseRecovery;
    const recoveryBlocked = new Promise((resolve) => {
      releaseRecovery = resolve;
    });
    let recoveries = 0;
    const first = createRuntimeRestartController({
      store,
      execute: async () => {
        throw new Error("recovery must not execute a restart");
      },
      recover: async () => {
        recoveries += 1;
        markRecoveryStarted();
        await recoveryBlocked;
      },
      logger: { error: () => {}, warn: () => {} },
    });

    const firstRecovery = first.recoverPending();
    await recoveryStarted;
    await store.updateGroup("restart-1", "completed");

    const second = createRuntimeRestartController({
      store,
      execute: async () => {
        throw new Error("recovery must not execute a restart");
      },
      recover: async () => {
        recoveries += 1;
      },
      logger: { error: () => {}, warn: () => {} },
    });
    const secondRecovery = await second.recoverPending();
    assert.equal(recoveries, 1);
    assert.deepEqual(secondRecovery.recoveredThreadIds, []);
    assert.deepEqual(secondRecovery.failedThreadIds, []);
    assert.deepEqual(secondRecovery.expectedRequestIds, ["restart-1"]);
    assert.deepEqual(secondRecovery.expectedThreadIds, ["thread-1"]);
    assert.equal(
      secondRecovery.recoveryOccurrenceId,
      "runtime-restart:restart-1",
    );

    releaseRecovery();
    assert.deepEqual((await firstRecovery).recoveredThreadIds, ["thread-1"]);
    assert.deepEqual(await store.recoverable(), []);
  });
});

test("execution outcome update does not revive a consumed recovery intent", async () => {
  await withStore(async (store) => {
    await store.accept(notification("restart-1"));
    await store.updateGroup("restart-1", "failed", "build failed");
    const controller = createRuntimeRestartController({
      store,
      execute: async () => {
        throw new Error("recovery must not execute a restart");
      },
      recover: async () => {},
      logger: { error: () => {}, warn: () => {} },
    });

    assert.deepEqual(
      (await controller.recoverPending()).recoveredThreadIds,
      ["thread-1"],
    );
    await store.updateGroup("restart-1", "completed");

    assert.deepEqual(await store.recoverable(), []);
    const duplicate = await store.accept(notification("restart-1"));
    assert.equal(duplicate.kind, "duplicate");
    assert.equal(duplicate.record.phase, "consumed");
    assert.equal(duplicate.record.outcomePhase, "failed");
  });
});

test("requestId accepts the byte limit and rejects an oversized value", async () => {
  await withStore(async (store) => {
    const accepted = await store.accept(
      notification("a".repeat(MAX_REQUEST_ID_BYTES)),
    );
    assert.equal(accepted.kind, "execute");

    const oversized = await store.accept(
      notification("é".repeat(MAX_REQUEST_ID_BYTES)),
    );
    assert.equal(oversized.kind, "invalid");
    assert.match(oversized.reason, /UTF-8 bytes/);
  });
});

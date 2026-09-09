const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");

const {
  createRuntimeRestartController,
  createRuntimeRestartIntentStore,
  expectedRuntimeRestartPrompt,
  MAX_REQUEST_ID_BYTES,
  recoverRuntimeRestartAfterThreadTerminal,
} = require("./runtimeRestartIntent.cjs");

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

function legacyNotification(requestId, mode, threadId = "thread-1") {
  const value = notification(requestId, threadId);
  value.params.mode = mode;
  return value;
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

test("legacy hot requests are rejected as unsupported before execution", async () => {
  await withStore(async (store) => {
    let executions = 0;
    const recovered = [];
    let releaseRecovery;
    const recoveryBlocked = new Promise((resolve) => {
      releaseRecovery = resolve;
    });
    let markRecoveryStarted;
    const recoveryStarted = new Promise((resolve) => {
      markRecoveryStarted = resolve;
    });
    const controller = createRuntimeRestartController({
      store,
      execute: async () => {
        executions += 1;
        return { ok: true };
      },
      recover: async (record) => {
        recovered.push(record);
        markRecoveryStarted();
        await recoveryBlocked;
      },
      logger: { error: () => {}, warn: () => {} },
    });

    const result = await controller.handle(
      legacyNotification("legacy-hot", "hot"),
    );

    assert.equal(result.ok, false);
    assert.equal(result.unsupported, true);
    assert.equal(result.persisted, true);
    assert.match(result.reason, /hot runtime refresh requests are unsupported/);
    assert.equal(executions, 0);
    assert.equal(recovered.length, 0);
    assert.equal((await store.recoverable())[0].phase, "failed");

    const activeResult = await recoverRuntimeRestartAfterThreadTerminal(
      controller,
      {
        method: "thread/status/changed",
        params: {
          threadId: "thread-1",
          lifecycleStatus: { type: "active", activeFlags: [] },
        },
      },
    );
    assert.deepEqual(activeResult.recoveredThreadIds, []);
    assert.equal(recovered.length, 0);

    const terminalNotification = {
      method: "thread/status/changed",
      params: {
        threadId: "thread-1",
        lifecycleStatus: {
          type: "final",
          result: { type: "completed" },
        },
      },
    };
    const terminalRecovery = recoverRuntimeRestartAfterThreadTerminal(
      controller,
      terminalNotification,
    );
    await recoveryStarted;
    const duplicateTerminalRecovery =
      await recoverRuntimeRestartAfterThreadTerminal(
        controller,
        terminalNotification,
      );
    assert.deepEqual(duplicateTerminalRecovery.recoveredThreadIds, []);
    assert.equal(recovered.length, 1);
    releaseRecovery();

    assert.deepEqual((await terminalRecovery).recoveredThreadIds, ["thread-1"]);
    assert.equal(recovered.length, 1);
    assert.equal(recovered[0].requestId, "legacy-hot");
    assert.equal(recovered[0].phase, "failed");
    assert.deepEqual(await store.recoverable(), []);
  });
});

test("malformed legacy hot requests are rejected before persistence", async () => {
  await withStore(async (store) => {
    const emptyRequestId = legacyNotification("", "hot");
    const missingThread = legacyNotification("legacy-hot", "hot", "");
    const oversized = legacyNotification(
      "é".repeat(MAX_REQUEST_ID_BYTES),
      "hot",
    );

    const emptyRequestIdResult = await store.accept(emptyRequestId);
    const missingThreadResult = await store.accept(missingThread);
    const oversizedResult = await store.accept(oversized);

    assert.equal(emptyRequestIdResult.kind, "invalid");
    assert.match(emptyRequestIdResult.reason, /requestId/);
    assert.equal(missingThreadResult.kind, "invalid");
    assert.match(missingThreadResult.reason, /requestedByThreadId/);
    assert.equal(oversizedResult.kind, "invalid");
    assert.match(oversizedResult.reason, /UTF-8 bytes/);
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
    assert.deepEqual(sameHostRecovery.expectedThreadIds, ["thread-1"]);

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
    assert.equal(recovered[0].completedByHostInstanceId, "host-1");
    assert.deepEqual(await store.recoverable(), []);
  });
});

test("legacy full requests are accepted and normalized to the single restart shape", async () => {
  await withStore(async (store) => {
    const accepted = await store.accept(
      legacyNotification("legacy-full", "full"),
    );

    assert.equal(accepted.kind, "execute");
    assert.equal(Object.hasOwn(accepted.record, "mode"), false);
  });
});

test("persisted legacy full intent is normalized during recovery", async () => {
  await withStore(
    async (store) => {
      const [record] = await store.recoverable();

      assert.equal(record.requestId, "legacy-full");
      assert.equal(record.phase, "completed");
      assert.equal(Object.hasOwn(record, "mode"), false);
    },
    {
      version: 1,
      records: [
        {
          requestId: "legacy-full",
          requestedByThreadId: "thread-1",
          mode: "full",
          phase: "completed",
        },
      ],
    },
  );
});

test("persisted legacy hot intent becomes a recoverable unsupported failure", async () => {
  await withStore(
    async (store) => {
      const [record] = await store.recoverable();

      assert.equal(record.requestId, "legacy-hot");
      assert.equal(record.phase, "failed");
      assert.equal(Object.hasOwn(record, "mode"), false);
      assert.match(record.error, /hot runtime refresh intent is unsupported/);
    },
    {
      version: 1,
      records: [
        {
          requestId: "legacy-hot",
          requestedByThreadId: "thread-1",
          mode: "hot",
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

test("failed execution is durable and recovery prompt forbids automatic restart", async () => {
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
    assert.match(expectedRuntimeRestartPrompt(record), /expected restart/);
    assert.match(expectedRuntimeRestartPrompt(record), /Runtime Capsule restart/);
    assert.doesNotMatch(expectedRuntimeRestartPrompt(record), /\(hot\)|\(full\)/);
    assert.match(
      expectedRuntimeRestartPrompt(record),
      /Do not call request_runtime_restart again automatically/,
    );

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
    assert.deepEqual(secondRecovery.expectedThreadIds, ["thread-1"]);

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

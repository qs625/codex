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
} = require("./runtimeRestartIntent.cjs");

async function withStore(run) {
  const directory = await fs.mkdtemp(
    path.join(os.tmpdir(), "runtime-restart-intent-test-"),
  );
  try {
    const store = createRuntimeRestartIntentStore(
      path.join(directory, "intents.json"),
      { fs },
    );
    await run(store);
  } finally {
    await fs.rm(directory, { recursive: true, force: true });
  }
}

function notification(requestId, mode = "hot", threadId = "thread-1") {
  return {
    method: "client/relaunch/requested",
    params: {
      requestId,
      mode,
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
    let markRecoveryAttempted;
    const recoveryAttempted = new Promise((resolve) => {
      markRecoveryAttempted = resolve;
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
        markRecoveryAttempted();
        throw new Error("thread is still finishing");
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

    releaseExecution({ ok: true, mode: "hot" });
    await recoveryAttempted;
    assert.equal((await store.recoverable())[0].phase, "completed");
  });
});

test("same-mode requests coalesce and cross-mode requests fail independently", async () => {
  await withStore(async (store) => {
    let releaseExecution;
    const execution = new Promise((resolve) => {
      releaseExecution = resolve;
    });
    const recovered = [];
    let markGroupRecovered;
    const groupRecovered = new Promise((resolve) => {
      markGroupRecovered = resolve;
    });
    let executions = 0;
    const controller = createRuntimeRestartController({
      store,
      execute: async () => {
        executions += 1;
        return execution;
      },
      recover: async (record) => {
        recovered.push(record);
        if (recovered.length === 3) {
          markGroupRecovered();
        }
      },
      logger: { error: () => {}, warn: () => {} },
    });

    assert.equal((await controller.handle(notification("one", "hot"))).ok, true);
    assert.equal(
      (await controller.handle(notification("two", "hot"))).coalesced,
      true,
    );
    const conflict = await controller.handle(notification("three", "full"));
    assert.equal(conflict.conflict, true);
    assert.equal(recovered[0].requestId, "three");
    assert.equal(executions, 1);
    assert.equal(
      (await store.recoverable()).find(
        (record) => record.requestId === "one",
      ).phase,
      "executing",
    );

    releaseExecution({ ok: true, mode: "hot" });
    await groupRecovered;
    await controller.waitForIdle();
    assert.equal(executions, 1);
    assert.deepEqual(
      recovered.map((record) => record.requestId).sort(),
      ["one", "three", "two"],
    );
  });
});

test("duplicate request id does not execute twice across controller instances", async () => {
  await withStore(async (store) => {
    let executions = 0;
    let markCompleted;
    const completed = new Promise((resolve) => {
      markCompleted = resolve;
    });
    const first = createRuntimeRestartController({
      store,
      execute: async () => {
        executions += 1;
        return { ok: true, mode: "hot" };
      },
      recover: async () => markCompleted(),
      logger: { error: () => {}, warn: () => {} },
    });
    await first.handle(notification("restart-1"));
    await completed;
    await first.waitForIdle();

    const second = createRuntimeRestartController({
      store,
      execute: async () => {
        executions += 1;
        return { ok: true, mode: "hot" };
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
    let markRecoveryAttempted;
    const recoveryAttempted = new Promise((resolve) => {
      markRecoveryAttempted = resolve;
    });
    const controller = createRuntimeRestartController({
      store,
      execute: async () => ({ ok: false, reason: "build failed" }),
      recover: async () => {
        markRecoveryAttempted();
        throw new Error("thread is still finishing");
      },
      logger: { error: () => {}, warn: () => {} },
    });
    await controller.handle(notification("restart-1"));
    await recoveryAttempted;

    const [record] = await store.recoverable();
    assert.equal(record.phase, "failed");
    assert.equal(record.error, "build failed");
    assert.match(expectedRuntimeRestartPrompt(record), /expected restart/);
    assert.match(
      expectedRuntimeRestartPrompt(record),
      /Do not call request_runtime_restart again automatically/,
    );

    const recoveryController = createRuntimeRestartController({
      store,
      execute: async () => {
        throw new Error("recovery must not execute a restart");
      },
      recover: async (pending) => recovered.push(pending),
      logger: { error: () => {}, warn: () => {} },
    });
    const recovery = await recoveryController.recoverPending();
    assert.deepEqual(recovery.recoveredThreadIds, ["thread-1"]);
    assert.equal(recovered[0].requestId, "restart-1");
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
        return { ok: true, mode: "hot" };
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

test("typed launcher recovery consumes the durable request group only after ack", async () => {
  await withStore(async (store) => {
    await store.accept(notification("restart-1"));
    await store.accept(notification("restart-2"));

    const consumed = await store.consumeRecoveredGroup("restart-1");

    assert.deepEqual(
      consumed.map((record) => record.requestId),
      ["restart-1", "restart-2"],
    );
    assert.deepEqual(await store.recoverable(), []);
  });
});

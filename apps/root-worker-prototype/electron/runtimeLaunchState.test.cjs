const assert = require("node:assert/strict");
const test = require("node:test");

const {
  buildLauncherRecoveryRecordParams,
  recordLauncherRecovery,
} = require("./runtimeLaunchState.cjs");

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

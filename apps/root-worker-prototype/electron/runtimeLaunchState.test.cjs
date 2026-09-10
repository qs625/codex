const assert = require("node:assert/strict");
const fs = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const {
  buildLauncherRecoveryRecordParams,
  recoverPayloadRuntimeFailureIfPresent,
  recordLauncherRecovery,
  writePayloadFailureEvidence,
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

test("payload recovery sends exact /self input, then consumes its evidence", async () => {
  const sent = [];
  const removals = [];
  const evidencePath = "/tmp/failure-evidence.json";
  const result = await recoverPayloadRuntimeFailureIfPresent({
    evidencePath,
    formatPayloadRuntimeRecoveryPrompt(params) {
      assert.deepEqual(params, {
        failedReleaseId: "release-current",
        reason: "renderer initialization failed",
        exitCode: null,
        signal: null,
      });
      return "运行时回退后恢复：请检查 release-current。";
    },
    fs: {
      async readFile() {
        return JSON.stringify({
          activationId: "payload-123",
          releaseId: "release-current",
          occurredAt: "2026-09-10T00:00:00.000Z",
          fallbackReleaseId: "release-seed",
          code: "payload_reported_error",
          message: "renderer initialization failed",
          details: { payloadPid: "123", source: "payload" },
        });
      },
      async rm(targetPath, options) {
        removals.push({ targetPath, options });
      },
    },
    async sendSelfCommand(text) {
      sent.push(text);
    },
  });

  assert.deepEqual(result, { recovered: true, evidence: true });
  assert.deepEqual(sent, ["运行时回退后恢复：请检查 release-current。"]);
  assert.deepEqual(removals, [
    { targetPath: evidencePath, options: { force: true } },
  ]);
});

test("payload recovery consumes evidence after exact /self input succeeds", async () => {
  let sent = 0;
  let removed = 0;
  const result = await recoverPayloadRuntimeFailureIfPresent({
    evidencePath: "/tmp/failure-evidence.json",
    formatPayloadRuntimeRecoveryPrompt() {
      return "运行时异常终止，请检查。";
    },
    fs: {
      async readFile() {
        return JSON.stringify({
          activationId: "runtime-1",
          releaseId: "release-current",
          occurredAt: "2026-09-10T00:00:00.000Z",
          fallbackReleaseId: "release-seed",
          code: "payload_exit_signal",
          message: "payload terminated by signal 15",
          details: { signal: "15" },
        });
      },
      async rm() {
        removed += 1;
      },
    },
    async sendSelfCommand() {
      sent += 1;
    },
  });

  assert.deepEqual(result, { recovered: true, evidence: true });
  assert.equal(sent, 1);
  assert.equal(removed, 1);
});

test("payload recovery retains evidence when /self input fails", async () => {
  let removed = 0;
  await assert.rejects(
    () =>
      recoverPayloadRuntimeFailureIfPresent({
        evidencePath: "/tmp/failure-evidence.json",
        formatPayloadRuntimeRecoveryPrompt() {
          return "运行时异常终止，请检查。";
        },
        fs: {
          async readFile() {
            return JSON.stringify({
              activationId: "runtime-1",
              releaseId: "release-current",
              occurredAt: "2026-09-10T00:00:00.000Z",
              fallbackReleaseId: "release-seed",
              code: "payload_exit_code",
              message: "payload exited with code 9",
              details: { exitCode: "9" },
            });
          },
          async rm() {
            removed += 1;
          },
        },
        async sendSelfCommand() {
          throw new Error("turn start failed");
        },
      }),
    /turn start failed/,
  );
  assert.equal(removed, 0);
});

test("payload recovery retries after evidence consumption fails", async () => {
  let removals = 0;
  const sent = [];
  const fsOps = {
    async readFile() {
      return JSON.stringify({
        activationId: "runtime-1",
        releaseId: "release-current",
        occurredAt: "2026-09-10T00:00:00.000Z",
        fallbackReleaseId: "release-seed",
        code: "payload_exit_code",
        message: "payload exited with code 9",
        details: { exitCode: "9" },
      });
    },
    async rm() {
      removals += 1;
      if (removals === 1) {
        throw new Error("consume failed");
      }
    },
  };
  const recover = () =>
    recoverPayloadRuntimeFailureIfPresent({
      evidencePath: "/tmp/failure-evidence.json",
      formatPayloadRuntimeRecoveryPrompt() {
        return "运行时异常终止，请检查。";
      },
      fs: fsOps,
      async sendSelfCommand(text) {
        sent.push(text);
      },
    });

  await assert.rejects(recover, /consume failed/);
  assert.deepEqual(await recover(), { recovered: true, evidence: true });
  assert.deepEqual(sent, ["运行时异常终止，请检查。", "运行时异常终止，请检查。"]);
  assert.equal(removals, 2);
});

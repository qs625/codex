const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");

const {
  buildLauncherRecoveryRecordParams,
  createRuntimeLaunchReadiness,
  readLauncherFailureEvidence,
  recordLauncherRecovery,
  recordLauncherRecoveryIfPresent,
} = require("./runtimeLaunchState.cjs");

test("readiness marker is written once after app-server and renderer are ready", async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "runtime-ready-test-"));
  const readyPath = path.join(root, "ready.json");
  try {
    const readiness = createRuntimeLaunchReadiness({
      createId: () => "nonce",
      env: {
        RUNTIME_CAPSULE_READY_PROTOCOL: "1",
        RUNTIME_CAPSULE_READY_PATH: readyPath,
        RUNTIME_CAPSULE_READY_TOKEN: "ready-token",
        RUNTIME_CAPSULE_RELEASE_ID: "release-1",
        RUNTIME_CAPSULE_LAUNCH_INSTANCE_ID: "launch-1",
        RUNTIME_CAPSULE_SPAWN_ATTEMPT_ID: "spawn-1",
        RUNTIME_CAPSULE_START_IDENTITY: "5678",
      },
      fs,
      pid: 4321,
    });

    assert.deepEqual(await readiness.markRendererReady(), { written: false });
    const result = await readiness.markAppServerReady();
    assert.equal(result.written, true);
    assert.deepEqual(JSON.parse(await fs.readFile(readyPath, "utf8")), {
      protocolVersion: 1,
      releaseId: "release-1",
      launchInstanceId: "launch-1",
      spawnAttemptId: "spawn-1",
      pid: 4321,
      startIdentity: 5678,
      token: "ready-token",
    });
    assert.equal(await readiness.markRendererReady(), result);
  } finally {
    await fs.rm(root, { force: true, recursive: true });
  }
});

test("readiness marker is not written when launch-bound identity is incomplete", async () => {
  const actions = [];
  const readiness = createRuntimeLaunchReadiness({
    env: {
      RUNTIME_CAPSULE_READY_PROTOCOL: "1",
      RUNTIME_CAPSULE_READY_PATH: "/tmp/ready.json",
      RUNTIME_CAPSULE_READY_TOKEN: "ready-token",
      RUNTIME_CAPSULE_RELEASE_ID: "release-1",
      RUNTIME_CAPSULE_LAUNCH_INSTANCE_ID: "launch-1",
      RUNTIME_CAPSULE_SPAWN_ATTEMPT_ID: "spawn-1",
    },
    fs: {
      async mkdir() {
        actions.push("mkdir");
      },
    },
    onReady(payload) {
      actions.push({ payload });
    },
    pid: 4321,
  });

  await readiness.markAppServerReady();
  const result = await readiness.markRendererReady();

  assert.deepEqual(result, { written: false, payload: null });
  assert.deepEqual(actions, [{ payload: null }]);
});

test("failure evidence maps generic Launcher identity to typed recovery params", () => {
  assert.deepEqual(
    buildLauncherRecoveryRecordParams({
      activationId: "activation-1",
      releaseId: "release-1",
      fallbackReleaseId: "release-0",
      code: "activation_rolled_back",
      message: "early crash",
      failed: { releaseId: "release-1" },
      fallback: { kind: "external", releaseId: "release-0" },
      occurredAt: "2024-09-09T00:00:00.000Z",
    }),
    {
      recoveryId: "activation-1:release-1",
      activationId: "activation-1",
      releaseId: "release-1",
      reason: "early crash",
      occurredAt: "2024-09-09T00:00:00.000Z",
      fallbackReleaseId: "release-0",
    },
  );
});

test("failure evidence without activation and release identity is rejected", () => {
  assert.throws(
    () =>
      buildLauncherRecoveryRecordParams({
        activationId: "activation-1",
        observedAtUnixMs: 1,
      }),
    /activation or release identity/,
  );
});

test("failure evidence reader treats a missing file as no recovery", async () => {
  assert.equal(
    await readLauncherFailureEvidence("/missing/failure-evidence.json", fs),
    null,
  );
});

const recoveryEvidence = {
  activationId: "activation-1",
  releaseId: "release-1",
  code: "activation_rolled_back",
  message: "early crash",
  occurredAt: "2024-09-09T00:00:00.000Z",
};

test("new self thread records recovery without subscribing again", async () => {
  const actions = [];

  const recorded = await recordLauncherRecovery({
    appServerClient: {
      async request(method, params) {
        actions.push({ method, params });
        return { recorded: true };
      },
    },
    evidence: recoveryEvidence,
    listResult: {
      materializedSelfThreadId: "current-self-root",
      selfProjectThreadId: "current-self-root",
    },
    runtimeLauncher: {
      async ackFailure(activationId) {
        actions.push({ activationId });
      },
    },
    async subscribeThread(threadId) {
      actions.push({ subscribeThreadId: threadId });
    },
  });

  assert.equal(recorded, true);
  assert.deepEqual(actions, [
    {
      method: "thread/clientRecovery/record",
      params: {
        threadId: "current-self-root",
        recoveryId: "activation-1:release-1",
        activationId: "activation-1",
        releaseId: "release-1",
        reason: "early crash",
        occurredAt: "2024-09-09T00:00:00.000Z",
      },
    },
    { activationId: "activation-1" },
  ]);
});

test("existing self thread is subscribed before recovery is recorded", async () => {
  const actions = [];

  const recorded = await recordLauncherRecovery({
    appServerClient: {
      async request(method, params) {
        actions.push({ method, threadId: params.threadId });
        return { recorded: false };
      },
    },
    evidence: recoveryEvidence,
    listResult: {
      materializedSelfThreadId: null,
      selfProjectThreadId: "current-self-root",
      threads: [
        {
          id: "other-self-root",
          name: "/self",
          cwd: "/Users/example/other-workspace",
        },
      ],
    },
    runtimeLauncher: {
      async ackFailure(activationId) {
        actions.push({ activationId });
      },
    },
    async subscribeThread(threadId) {
      actions.push({ subscribeThreadId: threadId });
    },
  });

  assert.equal(recorded, false);
  assert.deepEqual(actions, [
    { subscribeThreadId: "current-self-root" },
    {
      method: "thread/clientRecovery/record",
      threadId: "current-self-root",
    },
    { activationId: "activation-1" },
  ]);
});

test("malformed recovery evidence is retained without blocking startup", async () => {
  const actions = [];

  const result = await recordLauncherRecoveryIfPresent({
    appServerClient: {
      async request() {
        actions.push("record");
      },
    },
    evidencePath: "/tmp/malformed-recovery.json",
    fs: {
      async readFile() {
        return "{";
      },
    },
    async listThreads() {
      actions.push("list");
    },
    logger: {
      warn(...args) {
        actions.push({ warn: args });
      },
    },
    runtimeLauncher: {
      async ackFailure() {
        actions.push("ack");
      },
    },
    async subscribeThread() {
      actions.push("subscribe");
    },
  });

  assert.deepEqual(result, { recorded: false, evidence: true });
  assert.equal(actions.length, 1);
  assert.match(actions[0].warn[0], /evidence retained/);
});

test("unreadable recovery evidence is retained without blocking startup", async () => {
  const actions = [];
  const readError = Object.assign(new Error("permission denied"), {
    code: "EACCES",
  });

  const result = await recordLauncherRecoveryIfPresent({
    appServerClient: {
      async request() {
        actions.push("record");
      },
    },
    evidencePath: "/tmp/unreadable-recovery.json",
    fs: {
      async readFile() {
        throw readError;
      },
    },
    async listThreads() {
      actions.push("list");
    },
    logger: {
      warn(...args) {
        actions.push({ warn: args });
      },
    },
    runtimeLauncher: {
      async ackFailure() {
        actions.push("ack");
      },
    },
    async subscribeThread() {
      actions.push("subscribe");
    },
  });

  assert.deepEqual(result, { recorded: false, evidence: true });
  assert.equal(actions.length, 1);
  assert.match(actions[0].warn[1], /permission denied/);
});

test("valid recovery evidence records and acknowledges through the authoritative self thread", async () => {
  const actions = [];

  const result = await recordLauncherRecoveryIfPresent({
    appServerClient: {
      async request(method, params) {
        actions.push({ method, threadId: params.threadId });
        return { recorded: true };
      },
    },
    evidencePath: "/tmp/recovery.json",
    fs: {
      async readFile() {
        return JSON.stringify(recoveryEvidence);
      },
    },
    async listThreads() {
      actions.push("list");
      return {
        materializedSelfThreadId: null,
        selfProjectThreadId: "current-self-root",
      };
    },
    logger: {
      warn(...args) {
        actions.push({ warn: args });
      },
    },
    runtimeLauncher: {
      async ackFailure(activationId) {
        actions.push({ activationId });
      },
    },
    async subscribeThread(threadId) {
      actions.push({ subscribeThreadId: threadId });
    },
  });

  assert.deepEqual(result, { recorded: true, evidence: true });
  assert.deepEqual(actions, [
    "list",
    { subscribeThreadId: "current-self-root" },
    {
      method: "thread/clientRecovery/record",
      threadId: "current-self-root",
    },
    { activationId: "activation-1" },
  ]);
});

async function runBestEffortRecoveryScenario({
  evidence = recoveryEvidence,
  onAck,
  onRequest,
  onSubscribe,
} = {}) {
  const actions = [];
  const result = await recordLauncherRecoveryIfPresent({
    appServerClient: {
      async request() {
        actions.push("record");
        return onRequest ? onRequest() : { recorded: true };
      },
    },
    evidencePath: "/tmp/recovery.json",
    fs: {
      async readFile() {
        return JSON.stringify(evidence);
      },
    },
    async listThreads() {
      actions.push("list");
      return {
        materializedSelfThreadId: null,
        selfProjectThreadId: "current-self-root",
      };
    },
    logger: {
      warn() {
        actions.push("warn");
      },
    },
    runtimeLauncher: {
      async ackFailure() {
        actions.push("ack");
        return onAck?.();
      },
    },
    async subscribeThread() {
      actions.push("subscribe");
      return onSubscribe?.();
    },
  });
  return { actions, result };
}

test("non-object recovery evidence is retained without side effects", async () => {
  const { actions, result } = await runBestEffortRecoveryScenario({
    evidence: [],
  });

  assert.deepEqual(result, { recorded: false, evidence: true });
  assert.deepEqual(actions, ["warn"]);
});

test("recovery evidence with invalid fields is retained without record or ack", async () => {
  const { actions, result } = await runBestEffortRecoveryScenario({
    evidence: {
      activationId: "activation-1",
      occurredAt: "2024-09-09T00:00:00.000Z",
    },
  });

  assert.deepEqual(result, { recorded: false, evidence: true });
  assert.deepEqual(actions, ["list", "warn"]);
});

test("subscribe failure retains recovery evidence without record or ack", async () => {
  const { actions, result } = await runBestEffortRecoveryScenario({
    onSubscribe() {
      throw new Error("subscribe failed");
    },
  });

  assert.deepEqual(result, { recorded: false, evidence: true });
  assert.deepEqual(actions, ["list", "subscribe", "warn"]);
});

test("recovery RPC failure retains evidence without ack", async () => {
  const { actions, result } = await runBestEffortRecoveryScenario({
    onRequest() {
      throw new Error("record failed");
    },
  });

  assert.deepEqual(result, { recorded: false, evidence: true });
  assert.deepEqual(actions, ["list", "subscribe", "record", "warn"]);
});

test("invalid recovery RPC response retains evidence without ack", async () => {
  const { actions, result } = await runBestEffortRecoveryScenario({
    onRequest() {
      return {};
    },
  });

  assert.deepEqual(result, { recorded: false, evidence: true });
  assert.deepEqual(actions, ["list", "subscribe", "record", "warn"]);
});

test("ack failure retains recovery evidence after a successful record", async () => {
  const { actions, result } = await runBestEffortRecoveryScenario({
    onAck() {
      throw new Error("ack failed");
    },
  });

  assert.deepEqual(result, { recorded: false, evidence: true });
  assert.deepEqual(actions, [
    "list",
    "subscribe",
    "record",
    "ack",
    "warn",
  ]);
});

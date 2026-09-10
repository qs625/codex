const assert = require("node:assert/strict");
const test = require("node:test");

const {
  createClientRelaunchNotificationHandler,
  createInstalledArtifactUpdateLifecycleAdapter,
} = require("./appLifecycle.cjs");

test("complete candidate selection exits normally without a readiness handshake", async () => {
  const events = [];
  const exits = [];
  const lifecycle = createInstalledArtifactUpdateLifecycleAdapter({
    appExit(code) {
      exits.push(code);
    },
    async resolvePlan() {
      return {};
    },
    runtimeLauncher: {
      supported: true,
      async selectCandidate(request) {
        events.push({ type: "select", request });
        return {
          activationId: request.activationId,
          releaseId: `sha256:${"a".repeat(64)}`,
          control: { selected: { kind: "external" } },
        };
      },
    },
    async updateArtifacts() {
      return {
        ok: true,
        activationId: "candidate-1",
        incomingRoot: "/tmp/incoming/candidate-1",
        releaseId: `sha256:${"a".repeat(64)}`,
        manifest: { target: { os: "darwin", arch: "arm64" } },
      };
    },
    broadcastStatus(status) {
      events.push({ type: "status", status });
    },
  });

  const result = await lifecycle.requestUpdateAndRelaunch("update", "request-1");

  assert.equal(result.ok, true);
  assert.equal(result.relaunch.exitCode, 0);
  assert.deepEqual(exits, [0]);
  assert.deepEqual(events[0], {
    type: "status",
    status: {
      lifecycle: {
        type: "installedArtifactUpdate",
        phase: "preparing",
        requestId: "request-1",
        reason: "update",
      },
    },
  });
  assert.deepEqual(events[1], {
    type: "select",
    request: {
      activationId: "candidate-1",
      target: { os: "darwin", arch: "arm64" },
      reason: "update",
    },
  });
});

test("selection failure removes only the unselected incoming candidate", async () => {
  const cleaned = [];
  const lifecycle = createInstalledArtifactUpdateLifecycleAdapter({
    async resolvePlan() {
      return {};
    },
    runtimeLauncher: {
      supported: true,
      async selectCandidate() {
        throw new Error("invalid Capsule");
      },
    },
    async updateArtifacts() {
      return {
        ok: true,
        activationId: "candidate-1",
        incomingRoot: "/tmp/incoming/candidate-1",
        releaseId: "release-1",
        manifest: { target: { os: "darwin", arch: "arm64" } },
      };
    },
    async cleanupPreparedArtifact(target) {
      cleaned.push(target);
    },
  });

  const result = await lifecycle.requestUpdateAndRelaunch("update");

  assert.equal(result.ok, false);
  assert.match(result.reason, /invalid Capsule/);
  assert.deepEqual(cleaned, ["/tmp/incoming/candidate-1"]);
});

test("client relaunch rejects every obsolete mode shape before relaunch", async () => {
  let relaunches = 0;
  const handler = createClientRelaunchNotificationHandler({
    fullRelaunch: {
      async requestRelaunch() {
        relaunches += 1;
        return { ok: true, relaunching: true };
      },
    },
  });

  for (const mode of [undefined, null, "full", "hot"]) {
    const result = await handler({
      method: "client/relaunch/requested",
      params: { requestId: `obsolete-${String(mode)}`, mode },
    });
    assert.equal(result.ok, false);
    assert.match(result.reason, /do not support mode/);
  }
  assert.equal(relaunches, 0);
});

test("client relaunch without mode uses the current relaunch path", async () => {
  const reasons = [];
  const handler = createClientRelaunchNotificationHandler({
    fullRelaunch: {
      async requestRelaunch(reason) {
        reasons.push(reason);
        return { ok: true, relaunching: true, reason };
      },
    },
  });

  const result = await handler({
    method: "client/relaunch/requested",
    params: { requestId: "current-shape", reason: "更新 Runtime" },
  });

  assert.equal(result.ok, true);
  assert.equal(result.requestId, "current-shape");
  assert.deepEqual(reasons, ["更新 Runtime"]);
});

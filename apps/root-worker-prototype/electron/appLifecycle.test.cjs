const assert = require("node:assert/strict");
const test = require("node:test");

const {
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

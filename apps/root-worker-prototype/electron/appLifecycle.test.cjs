const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const {
  CAPSULE_SWITCH_EXIT_CODE,
  createClientRelaunchNotificationHandler,
  createInstalledArtifactUpdateLifecycleAdapter,
} = require("./appLifecycle.cjs");

const COMPUTER_USE_HELPER_APP_NAME = "Root Worker Computer Use";
const COMPUTER_USE_HELPER_BUNDLE_IDENTIFIER =
  "com.openai.root-worker-prototype.computer-use.dev";

function writeComputerUseHelperApp(appPath, marker = "helper") {
  const contentsDir = path.join(appPath, "Contents");
  const executable = path.join(contentsDir, "MacOS", COMPUTER_USE_HELPER_APP_NAME);
  fs.mkdirSync(path.dirname(executable), { recursive: true });
  fs.writeFileSync(
    path.join(contentsDir, "Info.plist"),
    `<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key>
  <string>${COMPUTER_USE_HELPER_BUNDLE_IDENTIFIER}</string>
</dict>
</plist>
`,
  );
  fs.writeFileSync(executable, marker, { mode: 0o755 });
}

function readComputerUseHelperExecutable(appPath) {
  return fs.readFileSync(
    path.join(appPath, "Contents", "MacOS", COMPUTER_USE_HELPER_APP_NAME),
    "utf8",
  );
}

test("complete candidate selection exits with the capsule switch code", async () => {
  const events = [];
  const exits = [];
  const lifecycle = createInstalledArtifactUpdateLifecycleAdapter({
    appExit(code) {
      events.push({ type: "exit", code });
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
    async gracefulShutdownAppServer(reason) {
      events.push({ type: "shutdown", reason });
    },
    broadcastStatus(status) {
      events.push({ type: "status", status });
    },
  });

  const result = await lifecycle.requestUpdateAndRelaunch(
    "update",
    "request-1",
    {
      async markExpectedRestartHandoffReady() {
        events.push({ type: "handoff-ready" });
      },
    },
  );

  assert.equal(result.ok, true);
  assert.equal(result.relaunch.exitCode, CAPSULE_SWITCH_EXIT_CODE);
  assert.deepEqual(exits, [CAPSULE_SWITCH_EXIT_CODE]);
  assert.deepEqual(
    events.map((event) => event.type),
    [
      "status",
      "select",
      "status",
      "status",
      "shutdown",
      "handoff-ready",
      "exit",
      "status",
    ],
  );
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
  assert.deepEqual(events[2], {
    type: "status",
    status: {
      lifecycle: {
        type: "installedArtifactUpdate",
        phase: "selected",
        activationId: "candidate-1",
        releaseId: `sha256:${"a".repeat(64)}`,
        requestId: "request-1",
        reason: "update",
      },
    },
  });
  assert.equal(events[3].status.lifecycle.phase, "shuttingDownAppServer");
  assert.deepEqual(events[4], { type: "shutdown", reason: "update" });
});

test("candidate selection fails before handoff when app-server graceful shutdown fails", async () => {
  let handoffMarkers = 0;
  let exits = 0;
  const lifecycle = createInstalledArtifactUpdateLifecycleAdapter({
    appExit() {
      exits += 1;
    },
    async resolvePlan() {
      return {};
    },
    runtimeLauncher: {
      supported: true,
      async selectCandidate(request) {
        return {
          activationId: request.activationId,
          releaseId: "release-1",
        };
      },
    },
    async updateArtifacts() {
      return {
        ok: true,
        activationId: "candidate-1",
        releaseId: "release-1",
      };
    },
    async gracefulShutdownAppServer() {
      throw new Error("shutdown timed out");
    },
  });

  const result = await lifecycle.requestUpdateAndRelaunch("update", null, {
    async markExpectedRestartHandoffReady() {
      handoffMarkers += 1;
    },
  });

  assert.equal(result.ok, false);
  assert.match(result.reason, /shutdown timed out/);
  assert.equal(handoffMarkers, 0);
  assert.equal(exits, 0);
});

test("candidate selection fails before handoff when app-server graceful shutdown returns not ok", async () => {
  let handoffMarkers = 0;
  let exits = 0;
  const lifecycle = createInstalledArtifactUpdateLifecycleAdapter({
    appExit() {
      exits += 1;
    },
    async resolvePlan() {
      return {};
    },
    runtimeLauncher: {
      supported: true,
      async selectCandidate(request) {
        return {
          activationId: request.activationId,
          releaseId: "release-1",
        };
      },
    },
    async updateArtifacts() {
      return {
        ok: true,
        activationId: "candidate-1",
        releaseId: "release-1",
      };
    },
    async gracefulShutdownAppServer() {
      return { ok: false, reason: "shutdown rejected" };
    },
  });

  const result = await lifecycle.requestUpdateAndRelaunch("update", null, {
    async markExpectedRestartHandoffReady() {
      handoffMarkers += 1;
    },
  });

  assert.equal(result.ok, false);
  assert.match(result.reason, /shutdown rejected/);
  assert.equal(handoffMarkers, 0);
  assert.equal(exits, 0);
});

test("selection failure removes only the unselected incoming candidate", async () => {
  const cleaned = [];
  let handoffMarkers = 0;
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

  const result = await lifecycle.requestUpdateAndRelaunch("update", null, {
    async markExpectedRestartHandoffReady() {
      handoffMarkers += 1;
    },
  });

  assert.equal(result.ok, false);
  assert.match(result.reason, /invalid Capsule/);
  assert.deepEqual(cleaned, ["/tmp/incoming/candidate-1"]);
  assert.equal(handoffMarkers, 0);
});

test("selection failure does not replace the stable Computer Use helper", async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "lifecycle-stable-helper-"));
  try {
    const cleaned = [];
    const stableHelperPath = path.join(
      root,
      "Applications",
      "Root Worker Computer Use.app",
    );
    writeComputerUseHelperApp(stableHelperPath, "old");
    const lifecycle = createInstalledArtifactUpdateLifecycleAdapter({
      async resolvePlan() {
        return {};
      },
      runtimeLauncher: {
        supported: true,
        async selectCandidate() {
          throw new Error("launcher unavailable");
        },
      },
      async updateArtifacts() {
        return {
          ok: true,
          activationId: "candidate-1",
          incomingRoot: path.join(root, "incoming", "candidate-1"),
          releaseId: "release-1",
          computerUseHelper: {
            status: "pending-selection",
            targetAppPath: stableHelperPath,
          },
          manifest: { target: { os: "darwin", arch: "arm64" } },
        };
      },
      async cleanupPreparedArtifact(target) {
        cleaned.push(target);
      },
    });

    const result = await lifecycle.requestUpdateAndRelaunch("update");

    assert.equal(result.ok, false);
    assert.match(result.reason, /launcher unavailable/);
    assert.deepEqual(cleaned, [path.join(root, "incoming", "candidate-1")]);
    assert.equal(readComputerUseHelperExecutable(stableHelperPath), "old");
  } finally {
    fs.rmSync(root, { force: true, recursive: true });
  }
});

test("selected candidate installs stable Computer Use helper from selected artifact", async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "lifecycle-stable-helper-"));
  try {
    const events = [];
    const commands = [];
    const stableHelperPath = path.join(
      root,
      "Applications",
      "Root Worker Computer Use.app",
    );
    const selectedRoot = path.join(root, "runtime-launcher", "artifacts", "digest");
    const payloadAppPath = path.join(
      selectedRoot,
      "payload",
      "Root Worker Runtime.app",
    );
    const selectedHelperPath = path.join(
      payloadAppPath,
      "Contents",
      "Resources",
      "computer-use-helper",
      "Root Worker Computer Use.app",
    );
    fs.mkdirSync(path.join(payloadAppPath, "Contents", "MacOS"), {
      recursive: true,
    });
    fs.writeFileSync(
      path.join(payloadAppPath, "Contents", "MacOS", "Root Worker Runtime"),
      "#!/bin/sh\n",
      { mode: 0o755 },
    );
    writeComputerUseHelperApp(selectedHelperPath, "new");
    const lifecycle = createInstalledArtifactUpdateLifecycleAdapter({
      appExit(code) {
        events.push({ type: "exit", code });
      },
      async resolvePlan() {
        return {};
      },
      runtimeLauncher: {
        supported: true,
        async selectCandidate(request) {
          return {
            activationId: request.activationId,
            releaseId: `sha256:${"a".repeat(64)}`,
            control: {
              selected: {
                kind: "external",
                capsule: { root: selectedRoot },
              },
            },
          };
        },
      },
      async updateArtifacts() {
        return {
          ok: true,
          activationId: "candidate-1",
          incomingRoot: path.join(root, "incoming", "candidate-1"),
          releaseId: `sha256:${"a".repeat(64)}`,
          computerUseHelper: {
            status: "pending-selection",
            targetAppPath: stableHelperPath,
          },
          manifest: { target: { os: "darwin", arch: "arm64" } },
        };
      },
      runCommand(command, args, options) {
        commands.push({ args, command, options });
      },
    });

    const result = await lifecycle.requestUpdateAndRelaunch("update");

    assert.equal(result.ok, true);
    assert.equal(result.computerUseHelper.status, "installed");
    assert.ok(
      commands.some(
        ({ args, command }) =>
          command === "codesign" &&
          args.length === 5 &&
          args.slice(0, 4).join(" ") === "--force --deep --sign -" &&
          args[4].endsWith(".staged"),
      ),
    );
    assert.deepEqual(events, [{ type: "exit", code: CAPSULE_SWITCH_EXIT_CODE }]);
    assert.ok(
      readComputerUseHelperExecutable(stableHelperPath).includes(
        path.join(selectedRoot, "payload"),
      ),
    );
  } finally {
    fs.rmSync(root, { force: true, recursive: true });
  }
});

test("disabled installed update does not mark expected restart handoff", async () => {
  let handoffMarkers = 0;
  const lifecycle = createInstalledArtifactUpdateLifecycleAdapter({
    async resolvePlan() {
      return { disabled: true, reason: "source workspace missing" };
    },
  });

  const result = await lifecycle.requestUpdateAndRelaunch("update", null, {
    async markExpectedRestartHandoffReady() {
      handoffMarkers += 1;
    },
  });

  assert.equal(result.disabled, true);
  assert.equal(handoffMarkers, 0);
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

test("client relaunch forwards expected restart handoff marker to installed update", async () => {
  let marker = null;
  const expectedMarker = async () => {};
  const handler = createClientRelaunchNotificationHandler({
    installedArtifactUpdate: {
      async requestUpdateAndRelaunch(_reason, _requestId, options) {
        marker = options?.markExpectedRestartHandoffReady;
        return { ok: true, relaunching: true };
      },
    },
  });

  const result = await handler({
    method: "client/relaunch/requested",
    params: {
      requestId: "current-shape",
      runtimeRestartHandoff: {
        markExpectedRestartHandoffReady: expectedMarker,
      },
    },
  });

  assert.equal(result.ok, true);
  assert.equal(marker, expectedMarker);
});

test("client relaunch fallback does not mark expected restart handoff", async () => {
  let markerCalls = 0;
  let fullRelaunches = 0;
  const handler = createClientRelaunchNotificationHandler({
    installedArtifactUpdate: {
      async requestUpdateAndRelaunch() {
        return { ok: false, unsupported: true };
      },
    },
    fullRelaunch: {
      async requestRelaunch() {
        fullRelaunches += 1;
        return { ok: true, relaunching: true };
      },
    },
  });

  const result = await handler({
    method: "client/relaunch/requested",
    params: {
      requestId: "fallback",
      runtimeRestartHandoff: {
        async markExpectedRestartHandoffReady() {
          markerCalls += 1;
        },
      },
    },
  });

  assert.equal(result.ok, true);
  assert.equal(fullRelaunches, 1);
  assert.equal(markerCalls, 0);
});

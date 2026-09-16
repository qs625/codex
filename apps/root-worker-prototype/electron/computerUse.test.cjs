const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const {
  buildAgentCursorPath,
  classifyComputerUseAction,
  createComputerUseManager,
  createMacNativeComputerUseClientWithAdapters,
  normalizeAction,
  resolveMacNativeComputerUseScriptPath,
} = require("./computerUse.cjs");

function fakeNativeClient(options = {}) {
  const actions = [];
  let observeCount = 0;
  const observations = options.observations ?? null;
  return {
    actions,
    async observe() {
      observeCount += 1;
      const observation = observations?.[Math.min(observeCount - 1, observations.length - 1)];
      const activeApp = observation?.activeApp ?? {
        name: options.activeAppName ?? "Firefox",
        bundleIdentifier: options.bundleIdentifier ?? "org.mozilla.firefox",
        processIdentifier: 42,
        window: {
          title: `Window ${observeCount}`,
          position: { x: 0, y: 0 },
          size: { width: 800, height: 600 },
        },
      };
      return {
        cursor: observation?.cursor ?? { x: 100 + observeCount, y: 200 + observeCount },
        activeApp,
        frontmostApp: observation?.frontmostApp ?? activeApp,
        targetApp: observation?.targetApp,
        targetVisibility: observation?.targetVisibility,
        accessibilityTrusted:
          observation?.accessibilityTrusted ?? options.accessibilityTrusted ?? true,
        screenshot: {
          path: `/tmp/screen-${observeCount}.png`,
          mimeType: "image/png",
          byteSize: 10,
          dataUrl: "data:image/png;base64,AA==",
        },
      };
    },
    async act(action) {
      actions.push(action);
    },
    async cleanup() {
      actions.push({ type: "cleanup" });
    },
  };
}

function fakeOverlayController() {
  const updates = [];
  return {
    updates,
    destroyed: 0,
    async update(payload) {
      updates.push(payload);
    },
    async destroy() {
      this.destroyed += 1;
    },
  };
}

test("computer use session starts with observe evidence", async () => {
  const nativeClient = fakeNativeClient();
  const overlayController = fakeOverlayController();
  const manager = createComputerUseManager({ nativeClient, overlayController });

  const state = await manager.startSession({ app: "Firefox" });

  assert.equal(state.status, "active");
  assert.equal(state.target.app, "Firefox");
  assert.equal(state.observation.sequence, 1);
  assert.equal(state.observation.activeApp.name, "Firefox");
  assert.equal(state.observation.screenshot.mimeType, "image/png");
  assert.deepEqual(state.systemCursor, { x: 101, y: 201 });
  assert.deepEqual(state.agentCursor, { x: 101, y: 201 });
  assert.deepEqual(state.cursor, state.agentCursor);
  assert.deepEqual(overlayController.updates.at(-1).agentCursor, state.agentCursor);
});

test("move updates agent cursor and overlay path without moving native cursor", async () => {
  const nativeClient = fakeNativeClient();
  const overlayController = fakeOverlayController();
  const manager = createComputerUseManager({ nativeClient, overlayController });
  await manager.startSession();

  const state = await manager.act({ type: "move", x: 320, y: 240 });

  assert.deepEqual(nativeClient.actions, []);
  assert.equal(state.status, "active");
  assert.deepEqual(state.agentCursor, { x: 320, y: 240 });
  assert.notDeepEqual(state.systemCursor, state.agentCursor);
  assert.equal(state.trace.length, 1);
  assert.equal(state.trace[0].status, "completed");
  assert.equal(state.trace[0].policy.kind, "low-risk");
  assert.ok(state.trace[0].agentCursorPath.length >= 2);
  assert.equal(state.observation.sequence, 3);
  assert.ok(
    state.pointerPath.some((point) => point.x === 320 && point.source === "move"),
  );
  const animatedUpdate = overlayController.updates.find(
    (update) => update.durationMs > 0,
  );
  assert.deepEqual(animatedUpdate.agentCursor, { x: 320, y: 240 });
  assert.deepEqual(animatedUpdate.targetBounds, {
    x: 0,
    y: 0,
    width: 800,
    height: 600,
  });
  assert.ok(animatedUpdate.pathSamples.length >= 2);
  assert.equal(animatedUpdate.pathSamples.at(-1).x, 320);
});

test("frontmost target without window bounds hides target-bound overlay", async () => {
  const activeAppWithoutBounds = {
    name: "Firefox",
    bundleIdentifier: "org.mozilla.firefox",
    processIdentifier: 42,
    window: { title: "Firefox without bounds" },
  };
  const nativeClient = fakeNativeClient({
    observations: [
      {
        activeApp: activeAppWithoutBounds,
        frontmostApp: activeAppWithoutBounds,
        targetApp: activeAppWithoutBounds,
        targetVisibility: "frontmost",
      },
    ],
  });
  const overlayController = fakeOverlayController();
  const manager = createComputerUseManager({ nativeClient, overlayController });
  await manager.startSession();

  const state = await manager.act({ type: "move", x: 320, y: 240 });

  assert.equal(state.targetVisibility, "frontmost");
  assert.equal(state.overlay.mode, "target-bound");
  assert.equal(state.overlay.visible, false);
  assert.match(state.overlay.reason, /bounds are unavailable/);
  assert.equal(overlayController.updates.length, 0);
  assert.ok(overlayController.destroyed >= 1);
});

test("frontmost target move outside target window hides target-bound overlay", async () => {
  const nativeClient = fakeNativeClient();
  const overlayController = fakeOverlayController();
  const manager = createComputerUseManager({ nativeClient, overlayController });
  await manager.startSession();
  const updatesBeforeMove = overlayController.updates.length;

  const state = await manager.act({ type: "move", x: 900, y: 700 });

  assert.equal(state.targetVisibility, "frontmost");
  assert.deepEqual(state.agentCursor, { x: 900, y: 700 });
  assert.equal(state.overlay.mode, "target-bound");
  assert.equal(state.overlay.visible, false);
  assert.match(state.overlay.reason, /outside the target window/);
  assert.equal(
    overlayController.updates.some(
      (update) => update.agentCursor?.x === 900 && update.agentCursor?.y === 700,
    ),
    false,
  );
  assert.equal(overlayController.updates.length, updatesBeforeMove + 1);
  assert.ok(overlayController.destroyed >= 1);
});

test("click moves the agent cursor before native desktop click", async () => {
  const nativeClient = fakeNativeClient();
  const overlayController = fakeOverlayController();
  const manager = createComputerUseManager({ nativeClient, overlayController });
  await manager.startSession();

  const state = await manager.act({ type: "click", x: 320, y: 240 });

  assert.deepEqual(nativeClient.actions, [{ type: "click", x: 320, y: 240 }]);
  assert.deepEqual(state.agentCursor, { x: 320, y: 240 });
  assert.ok(state.trace[0].agentCursorPath.length >= 2);
  assert.ok(
    overlayController.updates.some((update) => update.pathSamples?.at(-1)?.x === 320),
  );
});

test("dangerous and disabled actions are blocked before native side effects", async () => {
  const nativeClient = fakeNativeClient();
  const manager = createComputerUseManager({ nativeClient });
  await manager.startSession();

  const dangerous = await manager.act({ type: "type", text: "send password" });
  const disabled = await manager.act({
    type: "drag",
    from: { x: 1, y: 2 },
    to: { x: 3, y: 4 },
  });

  assert.deepEqual(nativeClient.actions, []);
  assert.equal(dangerous.trace[0].status, "blocked");
  assert.equal(dangerous.trace[0].policy.kind, "needs-confirmation");
  assert.equal(disabled.trace[1].status, "blocked");
  assert.equal(disabled.trace[1].policy.kind, "disabled");
});

test("policy keeps safe keyboard shortcuts available", () => {
  assert.deepEqual(classifyComputerUseAction({ type: "key", key: "t", modifiers: ["cmd"] }), {
    kind: "low-risk",
    allowed: true,
    reason: null,
  });
  assert.equal(
    classifyComputerUseAction(normalizeAction({
      type: "key",
      key: "q",
      modifiers: ["command"],
    })).allowed,
    false,
  );
  assert.equal(
    classifyComputerUseAction(normalizeAction({
      type: "key",
      key: "w",
      modifiers: ["meta"],
    })).allowed,
    false,
  );
});

test("actions are blocked when target app is not the observed active app", async () => {
  const nativeClient = fakeNativeClient({ activeAppName: "Finder", bundleIdentifier: "com.apple.finder" });
  const manager = createComputerUseManager({ nativeClient });
  await manager.startSession({ app: "org.mozilla.firefox" });

  const state = await manager.act({ type: "click", x: 10, y: 10 });

  assert.deepEqual(nativeClient.actions, []);
  assert.equal(state.trace[0].status, "blocked");
  assert.equal(state.trace[0].policy.kind, "target-mismatch");
});

test("action preflight blocks if the active app changed after observe", async () => {
  const nativeClient = fakeNativeClient({
    observations: [
      {
        activeApp: {
          name: "Firefox",
          bundleIdentifier: "org.mozilla.firefox",
          processIdentifier: 42,
          window: { title: "Firefox" },
        },
      },
      {
        activeApp: {
          name: "Finder",
          bundleIdentifier: "com.apple.finder",
          processIdentifier: 43,
          window: { title: "Finder" },
        },
      },
    ],
  });
  const manager = createComputerUseManager({ nativeClient });
  await manager.startSession({ app: "org.mozilla.firefox" });

  const state = await manager.act({ type: "click", x: 10, y: 10 });

  assert.deepEqual(nativeClient.actions, []);
  assert.equal(state.trace[0].status, "blocked");
  assert.equal(state.trace[0].policy.kind, "target-mismatch");
  assert.equal(state.observation.activeApp.name, "Finder");
});

test("background target observe does not confuse frontmost app with target app", async () => {
  const overlayController = fakeOverlayController();
  const nativeClient = fakeNativeClient({
    observations: [
      {
        activeApp: {
          name: "Firefox",
          bundleIdentifier: "org.mozilla.firefox",
          processIdentifier: 42,
          window: { title: "Firefox" },
        },
        targetApp: {
          name: "Firefox",
          bundleIdentifier: "org.mozilla.firefox",
          processIdentifier: 42,
          window: { title: "Firefox" },
        },
        targetVisibility: "frontmost",
      },
      {
        activeApp: {
          name: "Finder",
          bundleIdentifier: "com.apple.finder",
          processIdentifier: 43,
          window: { title: "Finder" },
        },
        targetApp: {
          name: "Firefox",
          bundleIdentifier: "org.mozilla.firefox",
          processIdentifier: 42,
          window: { title: "Firefox background" },
        },
        targetVisibility: "background",
      },
      {
        activeApp: {
          name: "Finder",
          bundleIdentifier: "com.apple.finder",
          processIdentifier: 43,
          window: { title: "Finder" },
        },
        targetApp: {
          name: "Firefox",
          bundleIdentifier: "org.mozilla.firefox",
          processIdentifier: 42,
          window: { title: "Firefox background" },
        },
        targetVisibility: "background",
      },
    ],
  });
  const manager = createComputerUseManager({ nativeClient, overlayController });
  await manager.startSession({ app: "org.mozilla.firefox" });

  const moved = await manager.act({ type: "move", x: 500, y: 400 });

  assert.deepEqual(nativeClient.actions, []);
  assert.equal(moved.frontmostApp.bundleIdentifier, "com.apple.finder");
  assert.equal(moved.targetApp.bundleIdentifier, "org.mozilla.firefox");
  assert.equal(moved.targetVisibility, "background");
  assert.equal(moved.overlay.mode, "target-bound");
  assert.equal(moved.overlay.visible, false);
  assert.match(moved.overlay.reason, /background target/);
  assert.match(moved.overlay.reason, /foreground app/);
  assert.match(moved.overlay.reason, /frontmost/);
  assert.ok(overlayController.destroyed >= 1);
  assert.deepEqual(moved.agentCursor, { x: 500, y: 400 });

  const clicked = await manager.act({ type: "click", x: 500, y: 400 });

  assert.deepEqual(nativeClient.actions, []);
  assert.equal(clicked.trace.at(-1).status, "blocked");
  assert.equal(clicked.trace.at(-1).policy.kind, "target-mismatch");
});

test("background target without window metadata does not inherit frontmost window", async () => {
  const nativeClient = fakeNativeClient({
    observations: [
      {
        activeApp: {
          name: "Finder",
          bundleIdentifier: "com.apple.finder",
          processIdentifier: 43,
          window: { title: "Finder should not become target" },
        },
        targetApp: {
          name: "Firefox",
          bundleIdentifier: "org.mozilla.firefox",
          processIdentifier: 42,
        },
        targetVisibility: "background",
      },
    ],
  });
  const manager = createComputerUseManager({ nativeClient });

  const state = await manager.startSession({ app: "org.mozilla.firefox" });

  assert.equal(state.frontmostApp.window.title, "Finder should not become target");
  assert.equal(state.targetApp.bundleIdentifier, "org.mozilla.firefox");
  assert.equal(state.targetVisibility, "background");
  assert.equal(state.target.window, null);
});

test("actions are blocked until observe proves accessibility permission", async () => {
  const nativeClient = fakeNativeClient({ accessibilityTrusted: false });
  const manager = createComputerUseManager({ nativeClient });
  await manager.startSession();

  const state = await manager.act({ type: "move", x: 10, y: 10 });

  assert.deepEqual(nativeClient.actions, []);
  assert.equal(state.trace[0].status, "blocked");
  assert.equal(state.trace[0].policy.kind, "needs-permission");
});

test("actions without a prior observation first preflight observe the desktop", async () => {
  const nativeClient = fakeNativeClient();
  const manager = createComputerUseManager({ nativeClient });

  const state = await manager.act({ type: "move", x: 10, y: 10 });

  assert.deepEqual(nativeClient.actions, []);
  assert.equal(state.trace[0].status, "completed");
  assert.equal(state.trace[0].policy.kind, "low-risk");
  assert.equal(state.observation.sequence, 2);
});

test("stop and cleanup destroy the agent cursor overlay", async () => {
  const nativeClient = fakeNativeClient();
  const overlayController = fakeOverlayController();
  const manager = createComputerUseManager({ nativeClient, overlayController });
  await manager.startSession();
  await manager.stopSession();
  await manager.cleanup();

  assert.equal(overlayController.destroyed, 2);
});

test("agent cursor path samples are bounded and end at the destination", () => {
  const samples = buildAgentCursorPath(
    { x: 0, y: 0 },
    { x: 100, y: 50 },
    { steps: 6, atMs: 1000, source: "move" },
  );

  assert.equal(samples.length, 6);
  assert.deepEqual(samples[0], { x: 0, y: 0, atMs: 1000, source: "move" });
  assert.equal(samples.at(-1).x, 100);
  assert.equal(samples.at(-1).y, 50);
});

test("native screenshot cache removes the previous observe evidence", async () => {
  const removed = [];
  let count = 0;
  const client = createMacNativeComputerUseClientWithAdapters({
    scriptPath: "/tmp/native.swift",
    tmpDir: "/tmp",
    async runNative() {
      return {
        cursor: { x: 1, y: 2 },
        activeApp: null,
        accessibilityTrusted: true,
      };
    },
    async screenshotCapture() {
      count += 1;
      return {
        path: `/tmp/screen-${count}.png`,
        mimeType: "image/png",
        byteSize: 1,
        dataUrl: "data:image/png;base64,AA==",
      };
    },
    async removeFile(file) {
      removed.push(file);
    },
  });

  await client.observe();
  await client.observe();
  await client.cleanup();

  assert.deepEqual(removed, [null, "/tmp/screen-1.png", "/tmp/screen-2.png"]);
});

test("native observe failure removes the just-captured screenshot", async () => {
  const removed = [];
  const client = createMacNativeComputerUseClientWithAdapters({
    scriptPath: "/tmp/native.swift",
    tmpDir: "/tmp",
    async runNative() {
      throw new Error("native observe failed");
    },
    async screenshotCapture() {
      return {
        path: "/tmp/screen-failed.png",
        mimeType: "image/png",
        byteSize: 1,
        dataUrl: "data:image/png;base64,AA==",
      };
    },
    async removeFile(file) {
      removed.push(file);
    },
  });

  await assert.rejects(() => client.observe(), /native observe failed/);

  assert.deepEqual(removed, ["/tmp/screen-failed.png"]);
});

test("mac native script resolver prefers packaged resource file", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "computer-use-resource-"));
  try {
    const resourcesPath = path.join(root, "Resources");
    const resourceScript = path.join(
      resourcesPath,
      "native",
      "computerUseMacNative.swift",
    );
    fs.mkdirSync(path.dirname(resourceScript), { recursive: true });
    fs.writeFileSync(resourceScript, "// bridge");

    assert.equal(
      resolveMacNativeComputerUseScriptPath({
        resourcesPath,
        sourceDirectory: "/repo/apps/root-worker-prototype/electron",
      }),
      resourceScript,
    );
  } finally {
    fs.rmSync(root, { force: true, recursive: true });
  }
});

test("mac native script resolver falls back to source tree path", () => {
  assert.equal(
    resolveMacNativeComputerUseScriptPath({
      resourcesPath: "/missing/Resources",
      sourceDirectory: "/repo/apps/root-worker-prototype/electron",
      fileExists: () => false,
    }),
    path.join(
      "/repo/apps/root-worker-prototype/electron",
      "computerUseMacNative.swift",
    ),
  );
});

test("computer use exposes headless IPC without adding a right panel view", () => {
  const electronDir = __dirname;
  const preload = fs.readFileSync(path.join(electronDir, "preload.cjs"), "utf8");
  const main = fs.readFileSync(path.join(electronDir, "main.cjs"), "utf8");
  const rightPanel = fs.readFileSync(
    path.join(electronDir, "../src/components/RightPanel.tsx"),
    "utf8",
  );
  const nativeBridge = fs.readFileSync(
    path.join(electronDir, "computerUseMacNative.swift"),
    "utf8",
  );

  assert.match(preload, /startComputerUse/);
  assert.match(preload, /actComputerUse/);
  assert.match(main, /codex:computerUse:act/);
  assert.match(main, /destroyComputerUseManager\(window\)/);
  assert.match(main, /createComputerUseOverlayController/);
  assert.doesNotMatch(nativeBridge, /postMouse\(\.mouseMoved/);
  assert.doesNotMatch(rightPanel, /Computer Use/);
  assert.doesNotMatch(rightPanel, /computerUse/i);
});

const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { join } = require("node:path");
const test = require("node:test");

const {
  classifyComputerUseAction,
  createComputerUseManager,
  createMacNativeComputerUseClientWithAdapters,
  normalizeAction,
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
      return {
        cursor: observation?.cursor ?? { x: 100 + observeCount, y: 200 + observeCount },
        activeApp: observation?.activeApp ?? {
          name: options.activeAppName ?? "Firefox",
          bundleIdentifier: options.bundleIdentifier ?? "org.mozilla.firefox",
          processIdentifier: 42,
          window: { title: `Window ${observeCount}` },
        },
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

test("computer use session starts with observe evidence", async () => {
  const nativeClient = fakeNativeClient();
  const manager = createComputerUseManager({ nativeClient });

  const state = await manager.startSession({ app: "Firefox" });

  assert.equal(state.status, "active");
  assert.equal(state.target.app, "Firefox");
  assert.equal(state.observation.sequence, 1);
  assert.equal(state.observation.activeApp.name, "Firefox");
  assert.equal(state.observation.screenshot.mimeType, "image/png");
  assert.deepEqual(state.cursor, { x: 101, y: 201 });
});

test("low-risk actions execute against native desktop and post-observe", async () => {
  const nativeClient = fakeNativeClient();
  const manager = createComputerUseManager({ nativeClient });
  await manager.startSession();

  const state = await manager.act({ type: "move", x: 320, y: 240 });

  assert.deepEqual(nativeClient.actions, [{ type: "move", x: 320, y: 240 }]);
  assert.equal(state.status, "active");
  assert.equal(state.trace.length, 1);
  assert.equal(state.trace[0].status, "completed");
  assert.equal(state.trace[0].policy.kind, "low-risk");
  assert.equal(state.observation.sequence, 3);
  assert.ok(
    state.pointerPath.some((point) => point.x === 320 && point.source === "move"),
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

  assert.deepEqual(nativeClient.actions, [{ type: "move", x: 10, y: 10 }]);
  assert.equal(state.trace[0].status, "completed");
  assert.equal(state.trace[0].policy.kind, "low-risk");
  assert.equal(state.observation.sequence, 2);
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

test("computer use exposes headless IPC without adding a right panel view", () => {
  const electronDir = __dirname;
  const preload = readFileSync(join(electronDir, "preload.cjs"), "utf8");
  const main = readFileSync(join(electronDir, "main.cjs"), "utf8");
  const rightPanel = readFileSync(
    join(electronDir, "../src/components/RightPanel.tsx"),
    "utf8",
  );

  assert.match(preload, /startComputerUse/);
  assert.match(preload, /actComputerUse/);
  assert.match(main, /codex:computerUse:act/);
  assert.match(main, /destroyComputerUseManager\(window\)/);
  assert.doesNotMatch(rightPanel, /Computer Use/);
  assert.doesNotMatch(rightPanel, /computerUse/i);
});

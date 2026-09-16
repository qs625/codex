import assert from "node:assert/strict";
import test from "node:test";
import { createRequire } from "node:module";

import {
  createComputerUseCliManagerFactory,
  parseComputerUseCliArgs,
  runComputerUseRequest,
} from "./morpheus-computer-use.mjs";

const require = createRequire(import.meta.url);
const {
  createComputerUseManager,
} = require("../apps/root-worker-prototype/electron/computerUse.cjs");

function fakeNativeClient(options = {}) {
  const actions = [];
  let observeCount = 0;
  return {
    actions,
    get observeCount() {
      return observeCount;
    },
    async observe() {
      observeCount += 1;
      const targetVisibility =
        options.targetVisibilitySequence?.[observeCount - 1] ??
        options.targetVisibility ??
        "frontmost";
      return {
        cursor: { x: 100 + observeCount, y: 200 + observeCount },
        systemCursor: { x: 100 + observeCount, y: 200 + observeCount },
        activeApp: {
          name: options.activeAppName ?? "Finder",
          bundleIdentifier: options.bundleIdentifier ?? "com.apple.finder",
          processIdentifier: 42,
          window: {
            title: `Window ${observeCount}`,
            position: { x: 100, y: 100 },
            size: { width: 900, height: 700 },
          },
        },
        frontmostApp: {
          name: options.activeAppName ?? "Finder",
          bundleIdentifier: options.bundleIdentifier ?? "com.apple.finder",
          processIdentifier: 42,
          window: {
            title: `Window ${observeCount}`,
            position: { x: 100, y: 100 },
            size: { width: 900, height: 700 },
          },
        },
        targetApp: {
          name: options.activeAppName ?? "Finder",
          bundleIdentifier: options.bundleIdentifier ?? "com.apple.finder",
          processIdentifier: 42,
          window: {
            title: `Window ${observeCount}`,
            position: { x: 100, y: 100 },
            size: { width: 900, height: 700 },
          },
        },
        targetVisibility,
        accessibilityTrusted: true,
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
      if (options.failActionType === action.type) {
        throw new Error(`${action.type} failed in native backend`);
      }
      if (action.type === "click" || action.type === "drag") {
        return {
          ok: true,
          systemCursorRestored: false,
          systemCursorBefore: { x: 101, y: 201 },
          systemCursorAfter: { x: 101, y: 201 },
        };
      }
      if (action.type === "type") {
        return { ok: true, characterCount: action.text.length };
      }
      if (action.type === "key") {
        return { ok: true, key: action.key, modifiers: action.modifiers ?? [] };
      }
      return { ok: true };
    },
    async activateTarget(payload) {
      actions.push({ type: "activate", ...payload });
      if (options.activationError) {
        throw new Error(options.activationError);
      }
      return options.activationResult ?? {
        activated: true,
        waitedMs: 50,
        targetApp: {
          name: options.activeAppName ?? "Finder",
          bundleIdentifier: options.bundleIdentifier ?? "com.apple.finder",
          processIdentifier: 42,
        },
      };
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
      return { available: true, visible: true };
    },
    async destroy() {
      this.destroyed += 1;
      return { available: true, visible: false, destroyed: true };
    },
  };
}

function managerHarness() {
  const nativeClient = fakeNativeClient();
  const manager = createComputerUseManager({ nativeClient });
  return { manager, nativeClient };
}

test("computer use CLI run keeps batch session state for observe move stop", async () => {
  const harness = managerHarness();

  const result = await runComputerUseRequest(
    parseComputerUseCliArgs([
      "run",
      "--app",
      "com.apple.finder",
      "--json",
      "--overlay-hold-ms",
      "0",
      "--actions",
      JSON.stringify([
        { type: "start" },
        { type: "observe" },
        { type: "move", x: 420, y: 360 },
        { type: "stop" },
      ]),
    ]),
    () => harness.manager,
  );

  assert.equal(result.ok, true);
  assert.equal(result.target.app, "com.apple.finder");
  assert.equal(result.results.length, 4);
  assert.equal(result.results[2].status, "completed");
  assert.deepEqual(result.results[2].state.agentCursor, { x: 420, y: 360 });
  assert.equal(result.results[2].state.trace.at(-1).action.type, "move");
  assert.equal(result.results[2].state.trace.at(-1).status, "completed");
  assert.ok(result.results[2].state.trace.at(-1).agentCursorPath.length >= 2);
  assert.ok(
    result.results[2].state.pointerPath.some(
      (point) => point.x === 420 && point.y === 360 && point.source === "move",
    ),
  );
  assert.equal(result.state.status, "stopped");
  assert.deepEqual(harness.nativeClient.actions, [{ type: "cleanup" }]);
});

test("computer use CLI runs click type key and drag by default", async () => {
  const nativeClient = fakeNativeClient();
  const overlayController = fakeOverlayController();

  const result = await runComputerUseRequest(
    parseComputerUseCliArgs([
      "run",
      "--app",
      "com.apple.finder",
      "--overlay-hold-ms",
      "0",
      "--actions",
      JSON.stringify([
        { type: "start" },
        { type: "click", x: 620, y: 460 },
        { type: "key", key: "Tab" },
        { type: "type", text: "hello" },
        { type: "drag", from: { x: 620, y: 460 }, to: { x: 640, y: 480 } },
        { type: "stop" },
      ]),
    ]),
    createComputerUseCliManagerFactory({
      nativeClient,
      overlayControllerFactory: async () => overlayController,
    }),
  );

  assert.equal(result.ok, true);
  assert.deepEqual(
    result.results.map((item) => item.status),
    ["completed", "completed", "completed", "completed", "completed", "completed"],
  );
  assert.deepEqual(result.policy.disabledActions, []);
  assert.deepEqual(
    result.policy.allowedActions,
    ["start", "observe", "move", "click", "key", "type", "drag", "stop"],
  );
  assert.deepEqual(nativeClient.actions, [
    { type: "click", x: 620, y: 460 },
    { type: "key", key: "Tab", modifiers: [] },
    { type: "type", text: "hello" },
    { type: "drag", from: { x: 620, y: 460 }, to: { x: 640, y: 480 } },
    { type: "cleanup" },
  ]);
  const clickTrace = result.results[1].state.trace.at(-1);
  assert.equal(clickTrace.action.type, "click");
  assert.equal(clickTrace.policy.kind, "side-effect");
  assert.equal(clickTrace.status, "completed");
  assert.equal(clickTrace.evidence.ok, true);
  assert.equal(clickTrace.evidence.systemCursorRestored, false);
  assert.ok(clickTrace.agentCursorPath.length >= 2);
  const keyTrace = result.results[2].state.trace.at(-1);
  assert.equal(keyTrace.action.type, "key");
  assert.equal(keyTrace.evidence.key, "Tab");
  assert.deepEqual(keyTrace.evidence.modifiers, []);
  const typeTrace = result.results[3].state.trace.at(-1);
  assert.equal(typeTrace.action.type, "type");
  assert.equal(typeTrace.evidence.characterCount, 5);
  const dragTrace = result.results[4].state.trace.at(-1);
  assert.equal(dragTrace.action.type, "drag");
  assert.equal(dragTrace.status, "completed");
  assert.equal(dragTrace.policy.kind, "side-effect");
  assert.ok(dragTrace.agentCursorPath.length >= 2);
  assert.equal(dragTrace.evidence.systemCursorRestored, false);
  assert.deepEqual(result.results[4].state.agentCursor, { x: 640, y: 480 });
  assert.equal(result.state.overlay.visible, false);
  assert.match(result.state.overlay.reason, /stopped/);
});

test("computer use CLI activates background targets before side effects", async () => {
  const nativeClient = fakeNativeClient({
    targetVisibilitySequence: ["background", "background", "frontmost", "frontmost"],
  });

  const result = await runComputerUseRequest(
    parseComputerUseCliArgs([
      "run",
      "--app",
      "com.apple.finder",
      "--overlay-hold-ms",
      "0",
      "--actions",
      JSON.stringify([{ type: "start" }, { type: "key", key: "Tab" }]),
    ]),
    createComputerUseCliManagerFactory({
      nativeClient,
      overlayControllerFactory: async () => fakeOverlayController(),
    }),
  );

  assert.equal(result.ok, true);
  assert.equal(result.results[1].status, "completed");
  assert.equal(result.results[1].state.trace.at(-1).policy.kind, "side-effect");
  assert.equal(result.results[1].state.trace.at(-1).activation.status, "completed");
  assert.equal(result.results[1].state.trace.at(-1).activation.before.targetVisibility, "background");
  assert.equal(result.results[1].state.trace.at(-1).activation.reobserved.targetVisibility, "frontmost");
  assert.deepEqual(nativeClient.actions.map((action) => action.type), ["activate", "key", "cleanup"]);
});

test("computer use CLI reports activation failures without native side effects", async () => {
  const nativeClient = fakeNativeClient({
    targetVisibility: "background",
    activationResult: {
      activated: false,
      reason: "Target app did not become frontmost after activation",
      waitedMs: 1000,
    },
  });

  const result = await runComputerUseRequest(
    parseComputerUseCliArgs([
      "run",
      "--app",
      "com.apple.finder",
      "--overlay-hold-ms",
      "0",
      "--actions",
      JSON.stringify([{ type: "key", key: "Tab" }]),
    ]),
    createComputerUseCliManagerFactory({
      nativeClient,
      overlayControllerFactory: async () => fakeOverlayController(),
    }),
  );

  assert.equal(result.ok, true);
  assert.equal(result.results[0].status, "failed");
  assert.match(result.results[0].error, /did not become frontmost/);
  assert.deepEqual(nativeClient.actions.map((action) => action.type), ["activate", "cleanup"]);
});

test("computer use CLI executes warning side effects with policy evidence", async () => {
  const nativeClient = fakeNativeClient();

  const result = await runComputerUseRequest(
    parseComputerUseCliArgs([
      "run",
      "--app",
      "com.apple.finder",
      "--overlay-hold-ms",
      "0",
      "--actions",
      JSON.stringify([
        { type: "type", text: "send password token" },
        { type: "key", key: "q", modifiers: ["cmd"] },
      ]),
    ]),
    createComputerUseCliManagerFactory({
      nativeClient,
      overlayControllerFactory: async () => fakeOverlayController(),
    }),
  );

  assert.equal(result.ok, true);
  assert.deepEqual(
    result.results.map((item) => item.status),
    ["completed", "completed"],
  );
  assert.deepEqual(
    result.results.map((item) => item.state.trace.at(-1).policy.kind),
    ["side-effect-warning", "side-effect-warning"],
  );
  assert.deepEqual(nativeClient.actions, [
    { type: "type", text: "send password token" },
    { type: "key", key: "q", modifiers: ["cmd"] },
    { type: "cleanup" },
  ]);
  assert.equal(
    result.results[0].state.trace.at(-1).evidence.characterCount,
    "send password token".length,
  );
  assert.equal(result.results[1].state.trace.at(-1).evidence.key, "q");
});

test("computer use CLI reports native side-effect failures without fake success", async () => {
  const nativeClient = fakeNativeClient({ failActionType: "key" });

  const result = await runComputerUseRequest(
    parseComputerUseCliArgs([
      "run",
      "--app",
      "com.apple.finder",
      "--overlay-hold-ms",
      "0",
      "--actions",
      JSON.stringify([{ type: "start" }, { type: "key", key: "Tab" }]),
    ]),
    createComputerUseCliManagerFactory({
      nativeClient,
      overlayControllerFactory: async () => fakeOverlayController(),
    }),
  );

  assert.equal(result.ok, true);
  assert.equal(result.results[1].status, "failed");
  assert.match(result.results[1].error, /key failed in native backend/);
  assert.equal(result.results[1].state.status, "error");
  assert.equal(result.results[1].state.trace.at(-1).status, "failed");
  assert.match(
    result.results[1].state.trace.at(-1).error,
    /key failed in native backend/,
  );
  assert.deepEqual(nativeClient.actions, [
    { type: "key", key: "Tab", modifiers: [] },
    { type: "cleanup" },
  ]);
});

test("computer use CLI wires a target-bound overlay for frontmost moves", async () => {
  const nativeClient = fakeNativeClient();
  const overlayController = fakeOverlayController();

  const result = await runComputerUseRequest(
    parseComputerUseCliArgs([
      "run",
      "--app",
      "com.apple.finder",
      "--overlay-hold-ms",
      "0",
      "--actions",
      JSON.stringify([
        { type: "start" },
        { type: "move", x: 620, y: 460 },
        { type: "stop" },
      ]),
    ]),
    createComputerUseCliManagerFactory({
      nativeClient,
      overlayControllerFactory: async () => overlayController,
    }),
  );

  assert.equal(result.ok, true);
  assert.equal(result.results[1].state.overlay.mode, "target-bound");
  assert.equal(result.results[1].state.overlay.visible, true);
  assert.deepEqual(result.results[1].state.agentCursor, { x: 620, y: 460 });
  assert.notDeepEqual(
    result.results[1].state.systemCursor,
    result.results[1].state.agentCursor,
  );
  assert.equal(overlayController.updates.length > 0, true);
  assert.deepEqual(overlayController.updates.at(-1).agentCursor, {
    x: 620,
    y: 460,
  });
  assert.deepEqual(overlayController.updates.at(-1).targetBounds, {
    x: 100,
    y: 100,
    width: 900,
    height: 700,
  });
  assert.equal(result.results[2].state.status, "stopped");
  assert.equal(result.results[2].state.overlay.mode, "target-bound");
  assert.equal(result.results[2].state.overlay.visible, false);
  assert.match(result.results[2].state.overlay.reason, /stopped/);
  assert.equal(result.state.status, "stopped");
  assert.equal(result.state.overlay.visible, false);
  assert.match(result.state.overlay.reason, /stopped/);
  assert.equal(overlayController.destroyed, 1);
  assert.deepEqual(nativeClient.actions, [{ type: "cleanup" }]);
});

test("computer use CLI keeps background target cursor hidden without drawing over foreground", async () => {
  const nativeClient = fakeNativeClient({ targetVisibility: "background" });
  const overlayController = fakeOverlayController();

  const result = await runComputerUseRequest(
    parseComputerUseCliArgs([
      "run",
      "--app",
      "com.apple.finder",
      "--overlay-hold-ms",
      "0",
      "--actions",
      JSON.stringify([{ type: "start" }, { type: "move", x: 620, y: 460 }]),
    ]),
    createComputerUseCliManagerFactory({
      nativeClient,
      overlayControllerFactory: async () => overlayController,
    }),
  );

  assert.equal(result.ok, true);
  assert.equal(result.results[1].state.overlay.mode, "target-bound");
  assert.equal(result.results[1].state.overlay.visible, false);
  assert.match(result.results[1].state.overlay.reason, /background target/);
  assert.match(result.results[1].state.overlay.reason, /foreground app/);
  assert.match(
    result.results[1].state.overlay.reason,
    /subsequent observe or move/,
  );
  assert.equal(result.state.overlay.mode, "target-bound");
  assert.equal(result.state.overlay.visible, false);
  assert.match(result.state.overlay.reason, /stopped/);
  assert.deepEqual(result.state.agentCursor, { x: 620, y: 460 });
  assert.equal(overlayController.updates.length, 0);
  assert.equal(overlayController.destroyed >= 1, true);
  assert.deepEqual(nativeClient.actions, [{ type: "cleanup" }]);
});

test("computer use CLI final state hides overlay after implicit batch cleanup", async () => {
  const nativeClient = fakeNativeClient();
  const overlayController = fakeOverlayController();

  const result = await runComputerUseRequest(
    parseComputerUseCliArgs([
      "run",
      "--app",
      "com.apple.finder",
      "--overlay-hold-ms",
      "0",
      "--actions",
      JSON.stringify([{ type: "start" }, { type: "move", x: 620, y: 460 }]),
    ]),
    createComputerUseCliManagerFactory({
      nativeClient,
      overlayControllerFactory: async () => overlayController,
    }),
  );

  assert.equal(result.ok, true);
  assert.equal(result.results[1].state.overlay.visible, true);
  assert.equal(result.state.status, "active");
  assert.equal(result.state.overlay.mode, "target-bound");
  assert.equal(result.state.overlay.visible, false);
  assert.match(result.state.overlay.reason, /stopped/);
  assert.equal(overlayController.destroyed, 1);
  assert.deepEqual(nativeClient.actions, [{ type: "cleanup" }]);
});

test("computer use CLI shows target-bound overlay on a later move after target becomes frontmost", async () => {
  const nativeClient = fakeNativeClient({
    targetVisibilitySequence: [
      "background",
      "background",
      "background",
      "frontmost",
      "frontmost",
      "frontmost",
    ],
  });
  const overlayController = fakeOverlayController();

  const result = await runComputerUseRequest(
    parseComputerUseCliArgs([
      "run",
      "--app",
      "com.apple.finder",
      "--overlay-hold-ms",
      "0",
      "--actions",
      JSON.stringify([
        { type: "start" },
        { type: "move", x: 620, y: 460 },
        { type: "observe" },
        { type: "move", x: 640, y: 480 },
      ]),
    ]),
    createComputerUseCliManagerFactory({
      nativeClient,
      overlayControllerFactory: async () => overlayController,
    }),
  );

  assert.equal(result.ok, true);
  assert.equal(result.results[1].state.targetVisibility, "background");
  assert.equal(result.results[1].state.overlay.visible, false);
  assert.equal(result.results[2].state.targetVisibility, "frontmost");
  assert.equal(result.results[2].state.overlay.visible, true);
  assert.equal(result.results[3].state.targetVisibility, "frontmost");
  assert.equal(result.results[3].state.overlay.visible, true);
  assert.deepEqual(result.results[3].state.agentCursor, { x: 640, y: 480 });
  assert.equal(
    overlayController.updates.every(
      (payload) => payload.targetVisibility === "frontmost",
    ),
    true,
  );
  assert.equal(
    overlayController.updates.some((payload) => payload.pathSamples.length > 0),
    true,
  );
  assert.deepEqual(nativeClient.actions, [{ type: "cleanup" }]);
});

test("computer use CLI hides frontmost target overlay when move is outside target window", async () => {
  const nativeClient = fakeNativeClient();
  const overlayController = fakeOverlayController();

  const result = await runComputerUseRequest(
    parseComputerUseCliArgs([
      "run",
      "--app",
      "com.apple.finder",
      "--overlay-hold-ms",
      "0",
      "--actions",
      JSON.stringify([{ type: "start" }, { type: "move", x: 1200, y: 900 }]),
    ]),
    createComputerUseCliManagerFactory({
      nativeClient,
      overlayControllerFactory: async () => overlayController,
    }),
  );

  assert.equal(result.ok, true);
  assert.equal(result.results[1].state.targetVisibility, "frontmost");
  assert.equal(result.results[1].state.overlay.mode, "target-bound");
  assert.equal(result.results[1].state.overlay.visible, false);
  assert.match(
    result.results[1].state.overlay.reason,
    /outside the target window/,
  );
  assert.equal(result.state.targetVisibility, "frontmost");
  assert.equal(result.state.overlay.mode, "target-bound");
  assert.equal(result.state.overlay.visible, false);
  assert.match(result.state.overlay.reason, /stopped/);
  assert.deepEqual(result.state.agentCursor, { x: 1200, y: 900 });
  assert.equal(
    overlayController.updates.some(
      (update) => update.agentCursor?.x === 1200 && update.agentCursor?.y === 900,
    ),
    false,
  );
  assert.equal(overlayController.updates.length, 2);
  assert.equal(overlayController.destroyed >= 1, true);
  assert.deepEqual(nativeClient.actions, [{ type: "cleanup" }]);
});

test("computer use CLI reports target-bound overlay limitation when helper is disabled", async () => {
  const nativeClient = fakeNativeClient();

  const result = await runComputerUseRequest(
    parseComputerUseCliArgs([
      "run",
      "--app",
      "com.apple.finder",
      "--no-overlay",
      "--overlay-hold-ms",
      "0",
      "--actions",
      JSON.stringify([{ type: "start" }, { type: "move", x: 620, y: 460 }]),
    ]),
    createComputerUseCliManagerFactory({
      nativeClient,
      overlayControllerFactory: async () => {
        throw new Error("overlay factory should not be called");
      },
    }),
  );

  assert.equal(result.ok, true);
  assert.equal(result.results[1].state.overlay.mode, "target-bound");
  assert.equal(result.results[1].state.overlay.visible, false);
  assert.match(
    result.results[1].state.overlay.reason,
    /overlay controller is unavailable/,
  );
  assert.equal(result.state.overlay.mode, "target-bound");
  assert.equal(result.state.overlay.visible, false);
  assert.match(result.state.overlay.reason, /stopped/);
  assert.deepEqual(result.state.agentCursor, { x: 620, y: 460 });
  assert.deepEqual(nativeClient.actions, [{ type: "cleanup" }]);
});

test("computer use CLI output keeps bounded screenshot data without stale temp path", async () => {
  const includedHarness = managerHarness();
  const included = await runComputerUseRequest(
    parseComputerUseCliArgs(["run", "--actions", '[{"type":"start"}]']),
    () => includedHarness.manager,
  );

  assert.equal(
    included.results[0].state.observation.screenshot.dataUrl,
    "data:image/png;base64,AA==",
  );
  assert.equal(included.results[0].state.observation.screenshot.path, undefined);
  assert.match(
    included.results[0].state.observation.screenshot.pathOmitted,
    /cleaned up after the CLI run/,
  );
  assert.equal(included.results[0].state.observation.screenshot.byteSize, 10);

  const omittedHarness = managerHarness();
  const omitted = await runComputerUseRequest(
    parseComputerUseCliArgs([
      "run",
      "--omit-screenshot-data",
      "--actions",
      '[{"type":"start"}]',
    ]),
    () => omittedHarness.manager,
  );

  assert.equal(omitted.results[0].state.observation.screenshot.dataUrl, undefined);
  assert.equal(
    omitted.results[0].state.observation.screenshot.dataUrlOmitted,
    true,
  );
  assert.equal(omitted.results[0].state.observation.screenshot.path, undefined);
});

test("computer use CLI rejects stateless move command form", () => {
  assert.throws(
    () => parseComputerUseCliArgs(["move", "--x", "1", "--y", "2"]),
    /Use `run --actions/,
  );
});

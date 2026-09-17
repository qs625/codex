import assert from "node:assert/strict";
import test from "node:test";
import { createRequire } from "node:module";
import { Readable } from "node:stream";

import {
  createComputerUseCliManagerFactory,
  parseComputerUseCliArgs,
  runComputerUseRepl,
  runComputerUseRequest,
} from "./morpheus-computer-use.mjs";

const require = createRequire(import.meta.url);
const {
  createComputerUseManager,
} = require("../apps/root-worker-prototype/electron/computerUse.cjs");

function fakeNativeClient(options = {}) {
  const actions = [];
  const observePayloads = [];
  let observeCount = 0;
  return {
    actions,
    observePayloads,
    get observeCount() {
      return observeCount;
    },
    async observe(payload = {}) {
      observePayloads.push(payload);
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
        perception: options.perception,
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
      if (
        ["click", "doubleClick", "rightClick", "drag", "scroll"].includes(
          action.type,
        )
      ) {
        return {
          ok: true,
          method: action.type === "scroll" ? "scroll-wheel" : "mouse-click",
          button: action.type === "rightClick" ? "right" : "left",
          clickCount: action.type === "doubleClick" ? 2 : 1,
          deltaX: action.deltaX,
          deltaY: action.deltaY,
          systemCursorRestored: false,
          systemCursorBefore: { x: 101, y: 201 },
          systemCursorAfter: { x: 101, y: 201 },
        };
      }
      if (action.type === "type") {
        return {
          ok: true,
          method: "pasteboard-cmd-v",
          characterCount: action.text.length,
          pasteboardRestored: true,
        };
      }
      if (action.type === "key" || action.type === "hotkey") {
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

async function runReplHarness({
  args = [],
  lines = [],
  nativeClient = null,
  overlayController = null,
  interruptAfterOutput = null,
} = {}) {
  const request = parseComputerUseCliArgs(["repl", ...args]);
  const stdout = [];
  const abortController = interruptAfterOutput === null ? null : new AbortController();
  const manager = createComputerUseManager({
    nativeClient: nativeClient ?? fakeNativeClient(),
    ...(overlayController ? { overlayController } : {}),
  });
  const result = await runComputerUseRepl(request, async () => manager, {
    input: Readable.from(lines.map((line) => `${line}\n`)),
    interruptSignal: abortController?.signal,
    writeStdout: (value) => {
      stdout.push(value.trimEnd());
      if (stdout.length === interruptAfterOutput) {
        abortController?.abort();
      }
    },
  });
  return { request, manager, result, stdout };
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
  assert.equal(Object.hasOwn(result.results[2], "traceItem"), false);
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

test("computer use REPL keeps one manager session across commands", async () => {
  const nativeClient = fakeNativeClient();
  const overlayController = fakeOverlayController();
  const { result, stdout } = await runReplHarness({
    args: ["--app", "com.apple.finder", "--omit-screenshot-data", "--overlay-hold-ms", "0"],
    lines: ["start", "observe", "move 420,360", "status", "exit"],
    nativeClient,
    overlayController,
  });

  assert.equal(result.ok, true);
  assert.equal(result.state.status, "stopped");
  assert.equal(stdout.length, 5);
  assert.match(stdout[2], /completed action=move/);
  assert.match(stdout[3], /status=active/);
  assert.match(stdout[4], /exit cleanup=completed/);
  assert.deepEqual(nativeClient.actions, [{ type: "cleanup" }]);
  assert.equal(overlayController.destroyed, 1);
});

test("computer use REPL emits JSON events with trace tail", async () => {
  const nativeClient = fakeNativeClient();
  const { result, stdout } = await runReplHarness({
    args: [
      "--app",
      "com.apple.finder",
      "--json",
      "--omit-screenshot-data",
      "--overlay-hold-ms",
      "0",
    ],
    lines: ["start", "click 30,40", "trace 1", "exit"],
    nativeClient,
  });

  const events = stdout.map((line) => JSON.parse(line));
  assert.equal(result.ok, true);
  assert.equal(events[1].type, "action");
  assert.equal(events[1].action.type, "click");
  assert.equal(events[1].status, "completed");
  assert.equal(events[1].traceTail.length, 1);
  assert.equal(events[1].traceTail[0].action.type, "click");
  assert.equal(events[2].type, "trace");
  assert.equal(events[2].traceTail.length, 1);
  assert.equal(events[3].cleanup.status, "completed");
  assert.deepEqual(nativeClient.actions, [
    { type: "click", x: 30, y: 40 },
    { type: "cleanup" },
  ]);
});

test("computer use REPL allows background pause without native act", async () => {
  const nativeClient = fakeNativeClient({ targetVisibility: "background" });
  const { result, stdout } = await runReplHarness({
    args: ["--app", "com.apple.finder", "--json", "--overlay-hold-ms", "0"],
    lines: ["start", '{"type":"pause","ms":0}', "exit"],
    nativeClient,
  });

  const pauseEvent = JSON.parse(stdout[1]);
  assert.equal(result.ok, true);
  assert.equal(pauseEvent.action.type, "wait");
  assert.equal(pauseEvent.status, "completed");
  assert.equal(pauseEvent.policy.kind, "low-risk");
  assert.equal(pauseEvent.evidence.waitMs, 0);
  assert.deepEqual(nativeClient.actions, [{ type: "cleanup" }]);
});

test("computer use REPL does not reuse wait evidence for stop action", async () => {
  const nativeClient = fakeNativeClient();
  const { result, stdout } = await runReplHarness({
    args: ["--app", "com.apple.finder", "--json", "--overlay-hold-ms", "0"],
    lines: ["start", "wait 30", "stop", "trace 1", "exit"],
    nativeClient,
  });

  const waitEvent = JSON.parse(stdout[1]);
  const stopEvent = JSON.parse(stdout[2]);
  const traceEvent = JSON.parse(stdout[3]);
  assert.equal(result.ok, true);
  assert.equal(waitEvent.action.type, "wait");
  assert.equal(waitEvent.evidence.waitMs, 30);
  assert.equal(stopEvent.action.type, "stop");
  assert.equal(stopEvent.evidence, null);
  assert.equal(stopEvent.policy, null);
  assert.equal(traceEvent.traceTail.length, 1);
  assert.equal(traceEvent.traceTail[0].action.type, "wait");
  assert.equal(traceEvent.traceTail[0].evidence.waitMs, 30);
  assert.deepEqual(nativeClient.actions, [{ type: "cleanup" }]);
});

test("computer use REPL keeps current evidence after trace reaches cap", async () => {
  const nativeClient = fakeNativeClient();
  const waitLines = [...Array.from({ length: 81 }, () => "wait 0"), "wait 7"];
  const { result, stdout } = await runReplHarness({
    args: ["--app", "com.apple.finder", "--json", "--overlay-hold-ms", "0"],
    lines: ["start", ...waitLines, "exit"],
    nativeClient,
  });

  const lastWaitEvent = JSON.parse(stdout[82]);
  assert.equal(result.ok, true);
  assert.equal(lastWaitEvent.action.type, "wait");
  assert.equal(lastWaitEvent.evidence.waitMs, 7);
  assert.equal(lastWaitEvent.traceTail.at(-1).action.type, "wait");
  assert.equal(lastWaitEvent.traceTail.at(-1).evidence.waitMs, 7);
  assert.deepEqual(nativeClient.actions, [{ type: "cleanup" }]);
});

test("computer use REPL cleans up on EOF", async () => {
  const nativeClient = fakeNativeClient();
  const { result, stdout } = await runReplHarness({
    args: ["--app", "com.apple.finder", "--json", "--overlay-hold-ms", "0"],
    lines: ["start", "move 10,20"],
    nativeClient,
  });

  const cleanupEvent = JSON.parse(stdout.at(-1));
  assert.equal(result.ok, true);
  assert.equal(cleanupEvent.command, "cleanup");
  assert.equal(cleanupEvent.reason, "eof");
  assert.equal(cleanupEvent.cleanup.status, "completed");
  assert.deepEqual(nativeClient.actions, [{ type: "cleanup" }]);
});

test("computer use REPL cleans up after failed side effect", async () => {
  const nativeClient = fakeNativeClient({ failActionType: "key" });
  const { result, stdout } = await runReplHarness({
    args: ["--app", "com.apple.finder", "--json", "--overlay-hold-ms", "0"],
    lines: ["start", "key Tab", "click 30,40"],
    nativeClient,
  });

  const failed = JSON.parse(stdout[1]);
  const cleanup = JSON.parse(stdout[2]);
  assert.equal(result.ok, false);
  assert.equal(stdout.length, 3);
  assert.equal(failed.status, "failed");
  assert.match(failed.error, /key failed/);
  assert.equal(cleanup.reason, "failed-action");
  assert.equal(cleanup.cleanup.status, "completed");
  assert.deepEqual(nativeClient.actions, [
    { type: "key", key: "Tab", modifiers: [] },
    { type: "cleanup" },
  ]);
});

test("computer use REPL cleans up after interrupt", async () => {
  const nativeClient = fakeNativeClient();
  const { result, stdout } = await runReplHarness({
    args: ["--app", "com.apple.finder", "--json", "--overlay-hold-ms", "0"],
    lines: ["start", "click 30,40"],
    nativeClient,
    interruptAfterOutput: 1,
  });

  const cleanup = JSON.parse(stdout[1]);
  assert.equal(result.ok, false);
  assert.equal(stdout.length, 2);
  assert.equal(cleanup.reason, "interrupt");
  assert.equal(cleanup.cleanup.status, "completed");
  assert.deepEqual(nativeClient.actions, [{ type: "cleanup" }]);
});

test("computer use CLI compiles shorthand flags into a run action batch", async () => {
  const request = parseComputerUseCliArgs([
    "run",
    "--app",
    "com.apple.finder",
    "--move",
    "10,20",
    "--click",
    "30,40",
    "--double-click",
    "50,60",
    "--right-click",
    "70,80",
    "--scroll",
    "90,100:0,-240",
    "--find-text",
    "Open",
    "--click-text",
    "Save",
    "--type",
    "hello",
    "--key",
    "cmd+s",
    "--hotkey",
    "cmd+shift+p",
    "--drag",
    "1,2:3,4",
    "--pause",
    "0",
    "--observe",
    "--overlay-hold-ms",
    "0",
  ]);

  assert.deepEqual(request.actions, [
    { type: "start" },
    { type: "move", x: 10, y: 20 },
    { type: "click", x: 30, y: 40 },
    { type: "doubleClick", x: 50, y: 60 },
    { type: "rightClick", x: 70, y: 80 },
    { type: "scroll", x: 90, y: 100, deltaX: 0, deltaY: -240 },
    { type: "findText", text: "Open" },
    { type: "clickText", text: "Save" },
    { type: "type", text: "hello" },
    { type: "key", key: "s", modifiers: ["cmd"] },
    { type: "hotkey", key: "p", modifiers: ["cmd", "shift"] },
    { type: "drag", from: { x: 1, y: 2 }, to: { x: 3, y: 4 } },
    { type: "wait", ms: 0 },
    { type: "observe" },
    { type: "stop" },
  ]);
});

test("computer use CLI passes bounded perception options to observe", async () => {
  const nativeClient = fakeNativeClient();
  const result = await runComputerUseRequest(
    parseComputerUseCliArgs([
      "run",
      "--app",
      "com.apple.finder",
      "--no-perception",
      "--perception-limit",
      "7",
      "--actions",
      JSON.stringify([{ type: "start" }]),
    ]),
    createComputerUseCliManagerFactory({
      nativeClient,
      overlayControllerFactory: async () => fakeOverlayController(),
    }),
  );

  assert.equal(result.ok, true);
  assert.equal(nativeClient.observePayloads[0].includePerception, false);
  assert.equal(nativeClient.observePayloads[0].perceptionLimit, 7);
});

test("computer use CLI rejects mixing shorthand flags with --actions", () => {
  assert.throws(
    () =>
      parseComputerUseCliArgs([
        "run",
        "--actions",
        JSON.stringify([{ type: "start" }]),
        "--type",
        "hello",
      ]),
    /either --actions or shorthand/,
  );
});

test("computer use CLI runs semantic side effects and wait by default", async () => {
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
        { type: "doubleClick", x: 621, y: 461 },
        { type: "rightClick", x: 622, y: 462 },
        { type: "scroll", x: 623, y: 463, deltaX: 0, deltaY: -120 },
        { type: "key", key: "Tab" },
        { type: "hotkey", key: "p", modifiers: ["cmd", "shift"] },
        { type: "type", text: "hello" },
        { type: "drag", from: { x: 620, y: 460 }, to: { x: 640, y: 480 } },
        { type: "wait", ms: 0 },
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
    [
      "completed",
      "completed",
      "completed",
      "completed",
      "completed",
      "completed",
      "completed",
      "completed",
      "completed",
      "completed",
      "completed",
    ],
  );
  assert.deepEqual(result.policy.disabledActions, []);
  assert.deepEqual(
    result.policy.allowedActions,
    [
      "start",
      "observe",
      "move",
      "click",
      "doubleClick",
      "rightClick",
      "scroll",
      "findText",
      "clickText",
      "key",
      "hotkey",
      "type",
      "drag",
      "wait",
      "pause",
      "stop",
    ],
  );
  assert.deepEqual(nativeClient.actions, [
    { type: "click", x: 620, y: 460 },
    { type: "doubleClick", x: 621, y: 461 },
    { type: "rightClick", x: 622, y: 462 },
    { type: "scroll", x: 623, y: 463, deltaX: 0, deltaY: -120 },
    { type: "key", key: "Tab", modifiers: [] },
    { type: "hotkey", key: "p", modifiers: ["cmd", "shift"] },
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
  const doubleClickTrace = result.results[2].state.trace.at(-1);
  assert.equal(doubleClickTrace.action.type, "doubleClick");
  assert.equal(doubleClickTrace.evidence.clickCount, 2);
  const rightClickTrace = result.results[3].state.trace.at(-1);
  assert.equal(rightClickTrace.action.type, "rightClick");
  assert.equal(rightClickTrace.evidence.button, "right");
  const scrollTrace = result.results[4].state.trace.at(-1);
  assert.equal(scrollTrace.action.type, "scroll");
  assert.equal(scrollTrace.evidence.deltaY, -120);
  const keyTrace = result.results[5].state.trace.at(-1);
  assert.equal(keyTrace.action.type, "key");
  assert.equal(keyTrace.evidence.key, "Tab");
  assert.deepEqual(keyTrace.evidence.modifiers, []);
  const hotkeyTrace = result.results[6].state.trace.at(-1);
  assert.equal(hotkeyTrace.action.type, "hotkey");
  assert.deepEqual(hotkeyTrace.evidence.modifiers, ["cmd", "shift"]);
  const typeTrace = result.results[7].state.trace.at(-1);
  assert.equal(typeTrace.action.type, "type");
  assert.equal(typeTrace.evidence.characterCount, 5);
  assert.equal(typeTrace.evidence.method, "pasteboard-cmd-v");
  assert.equal(typeTrace.evidence.pasteboardRestored, true);
  const dragTrace = result.results[8].state.trace.at(-1);
  assert.equal(dragTrace.action.type, "drag");
  assert.equal(dragTrace.status, "completed");
  assert.equal(dragTrace.policy.kind, "side-effect");
  assert.ok(dragTrace.agentCursorPath.length >= 2);
  assert.equal(dragTrace.evidence.systemCursorRestored, false);
  assert.deepEqual(result.results[8].state.agentCursor, { x: 640, y: 480 });
  const waitTrace = result.results[9].state.trace.at(-1);
  assert.equal(waitTrace.action.type, "wait");
  assert.equal(waitTrace.evidence.waitMs, 0);
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

test("computer use CLI allows JSON pause for background targets without native act", async () => {
  const nativeClient = fakeNativeClient({ targetVisibility: "background" });

  const result = await runComputerUseRequest(
    parseComputerUseCliArgs([
      "run",
      "--app",
      "com.apple.finder",
      "--overlay-hold-ms",
      "0",
      "--actions",
      JSON.stringify([{ type: "start" }, { type: "pause", ms: 0 }]),
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
  assert.equal(result.results[1].action.type, "wait");
  assert.equal(result.results[1].state.trace.at(-1).action.type, "wait");
  assert.equal(result.results[1].state.trace.at(-1).policy.kind, "low-risk");
  assert.equal(result.results[1].state.trace.at(-1).evidence.waitMs, 0);
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
  const cropScreenshot = {
    path: "/tmp/crop.png",
    mimeType: "image/png",
    byteSize: 5,
    dataUrl: "data:image/png;base64,CROP",
  };
  const includedNativeClient = fakeNativeClient({
    perception: {
      windowCrop: { screenshot: cropScreenshot },
      accessibilityElements: [],
    },
  });
  const includedHarness = {
    nativeClient: includedNativeClient,
    manager: createComputerUseManager({ nativeClient: includedNativeClient }),
  };
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
  assert.equal(
    included.results[0].state.observation.perception.windowCrop.screenshot.dataUrl,
    "data:image/png;base64,CROP",
  );
  assert.equal(
    included.results[0].state.observation.perception.windowCrop.screenshot.path,
    undefined,
  );

  const omittedNativeClient = fakeNativeClient({
    perception: {
      windowCrop: { screenshot: cropScreenshot },
      accessibilityElements: [],
    },
  });
  const omittedHarness = {
    nativeClient: omittedNativeClient,
    manager: createComputerUseManager({ nativeClient: omittedNativeClient }),
  };
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
  assert.equal(
    omitted.results[0].state.observation.perception.windowCrop.screenshot.dataUrl,
    undefined,
  );
  assert.equal(
    omitted.results[0].state.observation.perception.windowCrop.screenshot.dataUrlOmitted,
    true,
  );
  assert.equal(
    omitted.results[0].state.observation.perception.windowCrop.screenshot.path,
    undefined,
  );
});

test("computer use CLI rejects stateless move command form", () => {
  assert.throws(
    () => parseComputerUseCliArgs(["move", "--x", "1", "--y", "2"]),
    /Use `run --actions/,
  );
});

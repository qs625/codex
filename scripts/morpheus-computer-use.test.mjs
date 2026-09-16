import assert from "node:assert/strict";
import test from "node:test";
import { createRequire } from "node:module";

import {
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
      return {
        cursor: { x: 100 + observeCount, y: 200 + observeCount },
        systemCursor: { x: 100 + observeCount, y: 200 + observeCount },
        activeApp: {
          name: options.activeAppName ?? "Finder",
          bundleIdentifier: options.bundleIdentifier ?? "com.apple.finder",
          processIdentifier: 42,
          window: { title: `Window ${observeCount}` },
        },
        frontmostApp: {
          name: options.activeAppName ?? "Finder",
          bundleIdentifier: options.bundleIdentifier ?? "com.apple.finder",
          processIdentifier: 42,
          window: { title: `Window ${observeCount}` },
        },
        targetApp: {
          name: options.activeAppName ?? "Finder",
          bundleIdentifier: options.bundleIdentifier ?? "com.apple.finder",
          processIdentifier: 42,
          window: { title: `Window ${observeCount}` },
        },
        targetVisibility: "frontmost",
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
      return {};
    },
    async cleanup() {
      actions.push({ type: "cleanup" });
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

test("computer use CLI blocks click type key and drag before native side effects", async () => {
  const harness = managerHarness();

  const result = await runComputerUseRequest(
    parseComputerUseCliArgs([
      "run",
      "--actions",
      JSON.stringify([
        { type: "click", x: 10, y: 20 },
        { type: "type", text: "hello" },
        { type: "key", key: "t", modifiers: ["cmd"] },
        { type: "drag", from: { x: 1, y: 2 }, to: { x: 3, y: 4 } },
      ]),
    ]),
    () => harness.manager,
  );

  assert.equal(result.ok, true);
  assert.deepEqual(
    result.results.map((item) => item.status),
    ["blocked", "blocked", "blocked", "blocked"],
  );
  assert.deepEqual(
    result.results.map((item) => item.policy.kind),
    ["disabled", "disabled", "disabled", "disabled"],
  );
  assert.equal(harness.nativeClient.observeCount, 0);
  assert.deepEqual(harness.nativeClient.actions, []);
  assert.equal(result.state.status, "idle");
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

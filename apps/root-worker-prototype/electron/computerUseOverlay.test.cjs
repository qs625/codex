"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const {
  OVERLAY_HTML,
  createComputerUseOverlayController,
  displayUnionBounds,
  restoreRegularActivationPolicy,
} = require("./computerUseOverlay.cjs");

test("computer use overlay uses a click-through transparent window", async () => {
  const windows = [];
  const activationCalls = [];
  class FakeBrowserWindow {
    constructor(options) {
      this.options = options;
      this.bounds = options;
      this.closed = false;
      this.webContents = {
        scripts: [],
        async executeJavaScript(script) {
          this.scripts.push(script);
        },
      };
      windows.push(this);
    }
    setIgnoreMouseEvents(ignore, options) {
      this.ignoreMouseEvents = { ignore, options };
    }
    setAlwaysOnTop(enabled, level) {
      this.alwaysOnTop = { enabled, level };
    }
    setVisibleOnAllWorkspaces(enabled, options) {
      this.visibleOnAllWorkspaces = { enabled, options };
    }
    loadURL(url) {
      this.url = url;
      return Promise.resolve();
    }
    setBounds(bounds) {
      this.bounds = bounds;
    }
    showInactive() {
      this.shownInactive = true;
    }
    isDestroyed() {
      return this.closed;
    }
    close() {
      this.closed = true;
    }
  }
  const controller = createComputerUseOverlayController({
    BrowserWindow: FakeBrowserWindow,
    hostApp: {
      setActivationPolicy(policy) {
        activationCalls.push(["setActivationPolicy", policy]);
      },
      dock: {
        show() {
          activationCalls.push(["dock.show"]);
        },
      },
    },
    screen: {
      getAllDisplays: () => [
        { bounds: { x: 0, y: 0, width: 500, height: 400 } },
        { bounds: { x: 500, y: -100, width: 300, height: 500 } },
      ],
    },
  });

  await controller.update({
    agentCursor: { x: 240, y: 220 },
    durationMs: 180,
    pathSamples: [{ x: 100, y: 100 }, { x: 240, y: 220 }],
    targetBounds: { x: 10, y: 20, width: 300, height: 200 },
  });

  assert.equal(windows.length, 1);
  assert.equal(windows[0].options.transparent, true);
  assert.equal(windows[0].options.focusable, false);
  assert.equal(windows[0].options.skipTaskbar, true);
  assert.equal(windows[0].options.show, false);
  assert.deepEqual(windows[0].ignoreMouseEvents, {
    ignore: true,
    options: { forward: true },
  });
  assert.deepEqual(windows[0].bounds, { x: 0, y: -100, width: 800, height: 500 });
  assert.equal(windows[0].shownInactive, true);
  assert.match(windows[0].webContents.scripts[0], /agentCursor/);
  assert.match(windows[0].webContents.scripts[0], /pathSamples/);
  assert.match(windows[0].webContents.scripts[0], /targetBounds/);
  assert.deepEqual(activationCalls, [
    ["setActivationPolicy", "regular"],
    ["dock.show"],
    ["setActivationPolicy", "regular"],
    ["dock.show"],
  ]);

  await controller.destroy();
  assert.equal(windows[0].closed, true);
  assert.deepEqual(activationCalls.slice(-2), [
    ["setActivationPolicy", "regular"],
    ["dock.show"],
  ]);
});

test("computer use overlay can stay non-activating for CLI helper windows", async () => {
  const windows = [];
  const activationCalls = [];
  class FakeBrowserWindow {
    constructor(options) {
      this.options = options;
      this.closed = false;
      this.webContents = {
        scripts: [],
        async executeJavaScript(script) {
          this.scripts.push(script);
        },
      };
      windows.push(this);
    }
    setIgnoreMouseEvents(ignore, options) {
      this.ignoreMouseEvents = { ignore, options };
    }
    setAlwaysOnTop(enabled, level) {
      this.alwaysOnTop = { enabled, level };
    }
    setVisibleOnAllWorkspaces(enabled, options) {
      this.visibleOnAllWorkspaces = { enabled, options };
    }
    loadURL(url) {
      this.url = url;
      return Promise.resolve();
    }
    setBounds(bounds) {
      this.bounds = bounds;
    }
    showInactive() {
      this.shownInactive = true;
    }
    isDestroyed() {
      return this.closed;
    }
    close() {
      this.closed = true;
    }
  }
  const controller = createComputerUseOverlayController({
    BrowserWindow: FakeBrowserWindow,
    hostApp: {
      setActivationPolicy(policy) {
        activationCalls.push(["setActivationPolicy", policy]);
      },
      dock: {
        show() {
          activationCalls.push(["dock.show"]);
        },
      },
    },
    restoreActivationPolicy: false,
    screen: {
      getAllDisplays: () => [
        { bounds: { x: 0, y: 0, width: 500, height: 400 } },
      ],
    },
  });

  await controller.update({
    agentCursor: { x: 240, y: 220 },
    targetBounds: { x: 0, y: 0, width: 500, height: 400 },
  });
  await controller.destroy();

  assert.equal(windows.length, 1);
  assert.equal(windows[0].options.focusable, false);
  assert.equal(windows[0].options.show, false);
  assert.equal(windows[0].shownInactive, true);
  assert.deepEqual(activationCalls, []);
});

test("computer use overlay destroy restores regular app activation without a window", async () => {
  const activationCalls = [];
  const controller = createComputerUseOverlayController({
    hostApp: {
      setActivationPolicy(policy) {
        activationCalls.push(["setActivationPolicy", policy]);
      },
      dock: {
        show() {
          activationCalls.push(["dock.show"]);
        },
      },
    },
  });

  await controller.destroy();

  assert.deepEqual(activationCalls, [
    ["setActivationPolicy", "regular"],
    ["dock.show"],
  ]);
});

test("regular activation restore is a no-op without Electron app APIs", async () => {
  assert.doesNotThrow(() => restoreRegularActivationPolicy(null));
  assert.doesNotThrow(() => restoreRegularActivationPolicy({}));
  assert.doesNotThrow(() =>
    restoreRegularActivationPolicy({
      setActivationPolicy() {
        throw new Error("activation policy unavailable");
      },
      dock: {
        show() {
          throw new Error("dock unavailable");
        },
      },
    }),
  );
  assert.doesNotThrow(() =>
    restoreRegularActivationPolicy({
      dock: {
        show() {
          return Promise.reject(new Error("dock show rejected"));
        },
      },
    }),
  );
  await Promise.resolve();
});

test("computer use overlay bounds cover all displays", () => {
  assert.deepEqual(
    displayUnionBounds({
      getAllDisplays: () => [
        { bounds: { x: -100, y: 0, width: 100, height: 100 } },
        { bounds: { x: 0, y: 50, width: 200, height: 150 } },
      ],
    }),
    { x: -100, y: 0, width: 300, height: 200 },
  );
});

test("computer use overlay html contains cursor and trail surfaces", () => {
  assert.match(OVERLAY_HTML, /__setComputerUseCursor/);
  assert.match(OVERLAY_HTML, /<polyline id="path"/);
  assert.match(OVERLAY_HTML, /transition-property: left, top/);
});

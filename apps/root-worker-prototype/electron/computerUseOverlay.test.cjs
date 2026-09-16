"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const {
  OVERLAY_HTML,
  createComputerUseOverlayController,
  displayUnionBounds,
} = require("./computerUseOverlay.cjs");

test("computer use overlay uses a click-through transparent window", async () => {
  const windows = [];
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
  });

  assert.equal(windows.length, 1);
  assert.equal(windows[0].options.transparent, true);
  assert.equal(windows[0].options.focusable, false);
  assert.deepEqual(windows[0].ignoreMouseEvents, {
    ignore: true,
    options: { forward: true },
  });
  assert.deepEqual(windows[0].bounds, { x: 0, y: -100, width: 800, height: 500 });
  assert.equal(windows[0].shownInactive, true);
  assert.match(windows[0].webContents.scripts[0], /agentCursor/);
  assert.match(windows[0].webContents.scripts[0], /pathSamples/);

  await controller.destroy();
  assert.equal(windows[0].closed, true);
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

const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { join } = require("node:path");
const test = require("node:test");

test("main window uses minimal macOS titlebar chrome without going frameless", () => {
  const mainSource = readFileSync(join(__dirname, "main.cjs"), "utf8");
  const appSource = readFileSync(join(__dirname, "../src/App.tsx"), "utf8");
  const stylesSource = readFileSync(join(__dirname, "../src/styles.css"), "utf8");

  assert.match(mainSource, /process\.platform === "darwin"/);
  assert.match(mainSource, /titleBarStyle: "hiddenInset"/);
  assert.match(mainSource, /trafficLightPosition: \{ x: 14, y: 14 \}/);
  assert.doesNotMatch(mainSource, /frame:\s*false/);
  assert.match(appSource, /document\.body\.classList\.toggle\("macos-window-chrome", isMac\)/);
  assert.match(stylesSource, /body\.macos-window-chrome \.sidebar > \.sidebar-section-header:first-child/);
  assert.match(stylesSource, /padding-left: 86px;/);
  assert.match(stylesSource, /\.workspace-tab-strip \{[\s\S]*-webkit-app-region: drag;/);
  assert.match(stylesSource, /\.sidebar-section-header \{[\s\S]*-webkit-app-region: drag;/);
  assert.match(stylesSource, /button,[\s\S]*-webkit-app-region: no-drag;/);
  assert.match(stylesSource, /\.workspace-tab,[\s\S]*-webkit-app-region: no-drag;/);
  assert.match(stylesSource, /\.workspace-tab-close \{[\s\S]*-webkit-app-region: no-drag;/);
  assert.match(stylesSource, /\.panel-resizer,[\s\S]*-webkit-app-region: no-drag;/);
  assert.match(stylesSource, /\.thread-chip,[\s\S]*-webkit-app-region: no-drag;/);
  assert.match(stylesSource, /\.composer-shell,[\s\S]*-webkit-app-region: no-drag;/);
});

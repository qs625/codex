import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

test("TerminalPanel attaches xterm input forwarding before replay writes", () => {
  const source = readFileSync(join(__dirname, "TerminalPanel.tsx"), "utf8");
  const onDataIndex = source.indexOf("dataSubscription = terminal.onData");
  const onBinaryIndex = source.indexOf("binarySubscription = terminal.onBinary");
  const recordedSizeIndex = source.indexOf("resizeToRecordedReplaySize();");
  const replayWriteIndex = source.indexOf("terminal.write(decodeBase64(activeTab.replayBase64))");
  const postReplayFitIndex = source.indexOf(
    "if (shouldReplayAtRecordedSize) {\n          sendSize();",
    replayWriteIndex,
  );

  assert.notEqual(onDataIndex, -1);
  assert.notEqual(onBinaryIndex, -1);
  assert.notEqual(recordedSizeIndex, -1);
  assert.notEqual(replayWriteIndex, -1);
  assert.notEqual(postReplayFitIndex, -1);
  assert.ok(
    onDataIndex < replayWriteIndex,
    "xterm responses generated while parsing replay output must be forwarded to the PTY",
  );
  assert.ok(
    onBinaryIndex < replayWriteIndex,
    "binary xterm responses generated while parsing replay output must be forwarded to the PTY",
  );
  assert.ok(
    recordedSizeIndex < replayWriteIndex,
    "terminal replay should be written after restoring the recorded PTY size",
  );
  assert.ok(
    replayWriteIndex < postReplayFitIndex,
    "terminal should fit to the current viewport after replaying recorded output",
  );
});

test("TerminalPanel publishes fitted size as thread preferred terminal size", () => {
  const source = readFileSync(join(__dirname, "TerminalPanel.tsx"), "utf8");
  const fitIndex = source.indexOf("const next = { rows: terminal.rows, cols: terminal.cols };");
  const preferredIndex = source.indexOf("publishPreferredTerminalSize(next);");
  const resizeIndex = source.indexOf(".resizeTerminal({ tabId: activeTab.id, size: next })");
  const dedupeIndex = source.indexOf("previousPreferred?.threadId !== threadId");

  assert.notEqual(fitIndex, -1);
  assert.notEqual(preferredIndex, -1);
  assert.notEqual(resizeIndex, -1);
  assert.notEqual(dedupeIndex, -1);
  assert.ok(
    fitIndex < preferredIndex,
    "preferred terminal size must come from the fitted xterm viewport",
  );
});

test("TerminalPanel converts LF-only output only for read-only command views", () => {
  const source = readFileSync(join(__dirname, "TerminalPanel.tsx"), "utf8");
  const helperIndex = source.indexOf("function shouldConvertTerminalEol");
  const readOnlyIndex = source.indexOf("tab.readOnlyOutput === true", helperIndex);
  const fixedOutputIndex = source.indexOf("!tab.canWrite && !tab.canResize", helperIndex);
  const constructorIndex = source.indexOf("terminal = new Terminal({");
  const convertIndex = source.indexOf(
    "convertEol: shouldConvertTerminalEol(activeTab)",
    constructorIndex,
  );

  assert.notEqual(helperIndex, -1);
  assert.notEqual(readOnlyIndex, -1);
  assert.notEqual(fixedOutputIndex, -1);
  assert.notEqual(convertIndex, -1);
});

test("TerminalPanel uses a light xterm surface theme", () => {
  const source = readFileSync(join(__dirname, "TerminalPanel.tsx"), "utf8");
  const themeStart = source.indexOf("const TERMINAL_THEME = {");
  const themeEnd = source.indexOf("};", themeStart);
  const constructorIndex = source.indexOf("terminal = new Terminal({");
  const appliedThemeIndex = source.indexOf("theme: TERMINAL_THEME", constructorIndex);
  const themeSource = source.slice(themeStart, themeEnd);

  assert.notEqual(themeStart, -1);
  assert.notEqual(appliedThemeIndex, -1);
  assert.match(themeSource, /background: "#fbfaf8"/);
  assert.match(themeSource, /foreground: "#292524"/);
  assert.doesNotMatch(themeSource, /#111827/);
});

test("TerminalPanel viewport chrome uses the light panel surface", () => {
  const source = readFileSync(join(__dirname, "../styles.css"), "utf8");
  const shellStart = source.indexOf(".terminal-viewport-shell {");
  const emptyStart = source.indexOf(".terminal-empty {", shellStart);
  const emptyButtonEnd = source.indexOf(".preview-editor-shell {", emptyStart);
  const terminalViewportSource = source.slice(shellStart, emptyButtonEnd);

  assert.notEqual(shellStart, -1);
  assert.notEqual(emptyStart, -1);
  assert.match(terminalViewportSource, /background: #fbfaf8/);
  assert.match(terminalViewportSource, /background: #f5f3f0/);
  assert.match(terminalViewportSource, /scrollbar-color: #d6d3d1 #fbfaf8/);
  assert.doesNotMatch(terminalViewportSource, /#111827/);
});

test("TerminalPanel rebuilds xterm when fallback upgrades to live capabilities", () => {
  const source = readFileSync(join(__dirname, "TerminalPanel.tsx"), "utf8");
  const dependencyStart = source.indexOf("  }, [\n    activeTab?.id,");
  const dependencyEnd = source.indexOf("  ]);", dependencyStart);
  const dependencies = source.slice(dependencyStart, dependencyEnd);

  assert.match(dependencies, /activeTab\?\.canResize/);
  assert.match(dependencies, /activeTab\?\.canWrite/);
  assert.match(dependencies, /activeTab\?\.readOnlyOutput/);
});

test("TerminalPanel rebuilds xterm when a focused command receives replay output", () => {
  const source = readFileSync(join(__dirname, "TerminalPanel.tsx"), "utf8");
  const dependencyStart = source.indexOf("  }, [\n    activeTab?.id,");
  const dependencyEnd = source.indexOf("  ]);", dependencyStart);
  const dependencies = source.slice(dependencyStart, dependencyEnd);

  assert.match(dependencies, /activeTab\?\.replayBase64/);
  assert.match(dependencies, /activeTab\?\.replayTruncated/);
  assert.match(dependencies, /activeTab\?\.hasSequenceGap/);
  assert.match(dependencies, /activeTab\?\.replayThroughSequence/);
});

test("TerminalPanel publishes preferred size while idle with no active tab", () => {
  const source = readFileSync(join(__dirname, "TerminalPanel.tsx"), "utf8");
  const idleEffectIndex = source.indexOf(
    "if (activeTab) {\n      return undefined;",
  );
  const measureIndex = source.indexOf(
    "measureTerminalViewportSize(viewport, displayPreferences)",
  );
  const publishIndex = source.indexOf(
    "publishPreferredTerminalSize(next);",
    measureIndex,
  );
  const viewportIndex = source.indexOf(
    "className={`terminal-viewport ${activeTab ? \"\" : \"idle\"}`}",
  );
  const emptyIndex = source.indexOf("{!activeTab ? (");

  assert.notEqual(idleEffectIndex, -1);
  assert.notEqual(measureIndex, -1);
  assert.notEqual(publishIndex, -1);
  assert.notEqual(viewportIndex, -1);
  assert.notEqual(emptyIndex, -1);
  assert.ok(
    idleEffectIndex < measureIndex,
    "idle terminal effect should measure when there is no active tab",
  );
  assert.ok(
    viewportIndex < emptyIndex,
    "idle terminal viewport must remain mounted beneath the empty state",
  );
});

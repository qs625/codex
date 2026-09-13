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
  const initialFitIndex = source.indexOf("sendSize();");
  const replayWriteIndex = source.indexOf("terminal.write(decodeBase64(activeTab.replayBase64))");

  assert.notEqual(onDataIndex, -1);
  assert.notEqual(onBinaryIndex, -1);
  assert.notEqual(initialFitIndex, -1);
  assert.notEqual(replayWriteIndex, -1);
  assert.ok(
    onDataIndex < replayWriteIndex,
    "xterm responses generated while parsing replay output must be forwarded to the PTY",
  );
  assert.ok(
    onBinaryIndex < replayWriteIndex,
    "binary xterm responses generated while parsing replay output must be forwarded to the PTY",
  );
  assert.ok(
    initialFitIndex < replayWriteIndex,
    "terminal replay should be written after fitting to the current viewport",
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

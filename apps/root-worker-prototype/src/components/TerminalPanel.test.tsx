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
  const replayWriteIndex = source.indexOf("terminal.write(decodeBase64(activeTab.replayBase64))");

  assert.notEqual(onDataIndex, -1);
  assert.notEqual(onBinaryIndex, -1);
  assert.notEqual(replayWriteIndex, -1);
  assert.ok(
    onDataIndex < replayWriteIndex,
    "xterm responses generated while parsing replay output must be forwarded to the PTY",
  );
  assert.ok(
    onBinaryIndex < replayWriteIndex,
    "binary xterm responses generated while parsing replay output must be forwarded to the PTY",
  );
});

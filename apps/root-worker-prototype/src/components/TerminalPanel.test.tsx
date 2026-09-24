import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

test("TerminalPanel header uses the compact single-title panel style", () => {
  const source = readFileSync(join(__dirname, "TerminalPanel.tsx"), "utf8");

  assert.match(source, /className="panel-content-header terminal-header"/);
  assert.doesNotMatch(source, /panel-eyebrow/);
  assert.doesNotMatch(source, /<p title=\{activeTab\?\.cwd\}>/);
});

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

test("TerminalPanel keeps xterm runtime modules behind one dynamic import boundary", () => {
  const source = readFileSync(join(__dirname, "TerminalPanel.tsx"), "utf8");
  const importBlockIndex = source.indexOf(
    'Promise.all([import("@xterm/xterm"), import("@xterm/addon-fit")])',
  );
  const runtimeAddonImports = source.match(/import\("@xterm\/addon-fit"\)/g) ?? [];

  assert.equal(
    source.includes('import { FitAddon } from "@xterm/addon-fit"'),
    false,
  );
  assert.notEqual(importBlockIndex, -1);
  assert.equal(runtimeAddonImports.length, 1);
  assert.match(source, /import type \{ FitAddon as XTermFitAddon \} from "@xterm\/addon-fit"/);
});

test("TerminalPanel publishes fitted size as thread preferred terminal size", () => {
  const source = readFileSync(join(__dirname, "TerminalPanel.tsx"), "utf8");
  const fitIndex = source.indexOf("const next = { rows: terminal.rows, cols: terminal.cols };");
  const preferredIndex = source.indexOf("publishPreferredTerminalSize(next);");
  const resizeIndex = source.indexOf(".resizeTerminal({ tabId: mountedTabId, size: next })");
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
});

test("TerminalPanel does not rebuild xterm for streaming output deltas", () => {
  const source = readFileSync(join(__dirname, "TerminalPanel.tsx"), "utf8");
  const dependencyStart = source.indexOf("  }, [\n    activeTab?.id,");
  const dependencyEnd = source.indexOf("  ]);", dependencyStart);
  const dependencies = source.slice(dependencyStart, dependencyEnd);
  const directDeltaWriteIndex = source.indexOf(
    "terminalRef.current?.write(decodeBase64(event.deltaBase64));",
  );

  assert.notEqual(directDeltaWriteIndex, -1);
  assert.doesNotMatch(dependencies, /activeTab\?\.replayThroughSequence/);
  assert.doesNotMatch(dependencies, /activeTab\?\.status/);
});

test("TerminalPanel input and resize handlers read the latest active tab runtime", () => {
  const source = readFileSync(join(__dirname, "TerminalPanel.tsx"), "utf8");
  const runtimeRefIndex = source.indexOf("const activeTabRuntimeRef = useRef");
  const runtimeUpdateIndex = source.indexOf("activeTabRuntimeRef.current = activeTab");
  const sendSizeIndex = source.indexOf("sendSize = () => {");
  const resizeCurrentTabIndex = source.indexOf(
    "const currentTab = activeTabRuntimeRef.current;",
    sendSizeIndex,
  );
  const resizeGuardIndex = source.indexOf(
    "currentTab?.id === mountedTabId",
    resizeCurrentTabIndex,
  );
  const dataIndex = source.indexOf("dataSubscription = terminal.onData");
  const dataCurrentTabIndex = source.indexOf(
    "const currentTab = activeTabRuntimeRef.current;",
    dataIndex,
  );
  const dataGuardIndex = source.indexOf(
    "currentTab?.id !== mountedTabId",
    dataCurrentTabIndex,
  );
  const binaryIndex = source.indexOf("binarySubscription = terminal.onBinary");
  const binaryCurrentTabIndex = source.indexOf(
    "const currentTab = activeTabRuntimeRef.current;",
    binaryIndex,
  );
  const binaryGuardIndex = source.indexOf(
    "currentTab?.id !== mountedTabId",
    binaryCurrentTabIndex,
  );

  assert.notEqual(runtimeRefIndex, -1);
  assert.notEqual(runtimeUpdateIndex, -1);
  assert.notEqual(resizeCurrentTabIndex, -1);
  assert.notEqual(resizeGuardIndex, -1);
  assert.notEqual(dataCurrentTabIndex, -1);
  assert.notEqual(dataGuardIndex, -1);
  assert.notEqual(binaryCurrentTabIndex, -1);
  assert.notEqual(binaryGuardIndex, -1);
});

test("TerminalPanel writes terminal exit markers without remounting xterm", () => {
  const source = readFileSync(join(__dirname, "TerminalPanel.tsx"), "utf8");
  const helperIndex = source.indexOf("function writeTerminalStatusMarker");
  const mountMarkerIndex = source.indexOf(
    "writeTerminalStatusMarker(terminal, activeTab, terminalStatusMarkerRef);",
  );
  const statusEffectIndex = source.indexOf(
    "terminal.options.cursorBlink = isInteractive(activeTab.status);",
  );
  const lostMarkerIndex = source.indexOf(
    "[Session disconnected from the runtime.]",
    helperIndex,
  );
  const exitMarkerIndex = source.indexOf("[Process exited", helperIndex);
  const cleanupMarkerIndex = source.indexOf(
    "terminalStatusMarkerRef.current = null;",
    source.indexOf("return () => {", mountMarkerIndex),
  );

  assert.notEqual(helperIndex, -1);
  assert.notEqual(mountMarkerIndex, -1);
  assert.notEqual(statusEffectIndex, -1);
  assert.notEqual(lostMarkerIndex, -1);
  assert.notEqual(exitMarkerIndex, -1);
  assert.notEqual(cleanupMarkerIndex, -1);
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

test("TerminalPanel focuses xterm only from explicit focus requests", () => {
  const source = readFileSync(join(__dirname, "TerminalPanel.tsx"), "utf8");
  const requestHelperIndex = source.indexOf(
    "const requestTerminalViewportFocus = useCallback",
  );
  const applyHelperIndex = source.indexOf("const applyPendingTerminalFocus = useCallback");
  const guardedFocusIndex = source.indexOf(
    "lastAppliedTerminalFocusTokenRef.current = request.token;\n    terminal.focus();",
    applyHelperIndex,
  );
  const targetGuardIndex = source.indexOf(
    "shouldApplyTerminalViewportFocusRequest({",
    applyHelperIndex,
  );
  const mountFocusIndex = source.indexOf("applyPendingTerminalFocus();");
  const unconditionalMountFocusIndex = source.indexOf("terminal?.focus();");
  const focusRequestEffectIndex = source.indexOf(
    "requestTerminalViewportFocus(nextState.activeTabId);",
  );
  const newShellFocusIndex = source.indexOf(
    "requestTerminalViewportFocus(nextState.activeTabId);",
    source.indexOf("const createTerminal = () => {"),
  );
  const liveCommandFocusIndex = source.indexOf(
    "const focusLiveCommand = (command: (typeof liveCommands)[number]) => {",
  );
  const tabClickFocusIndex = source.indexOf(
    "requestTerminalViewportFocus(tab.id);",
    source.indexOf(".selectTerminalTab(tab.id)"),
  );

  assert.notEqual(requestHelperIndex, -1);
  assert.notEqual(applyHelperIndex, -1);
  assert.notEqual(guardedFocusIndex, -1);
  assert.notEqual(targetGuardIndex, -1);
  assert.notEqual(mountFocusIndex, -1);
  assert.equal(
    unconditionalMountFocusIndex,
    -1,
    "terminal mount/replay updates must not unconditionally steal focus",
  );
  assert.notEqual(focusRequestEffectIndex, -1);
  assert.notEqual(newShellFocusIndex, -1);
  assert.notEqual(liveCommandFocusIndex, -1);
  assert.notEqual(
    source.indexOf(
      "requestTerminalViewportFocus(nextState.activeTabId);",
      liveCommandFocusIndex,
    ),
    -1,
  );
  assert.notEqual(tabClickFocusIndex, -1);
});

test("TerminalPanel passive state refreshes do not request xterm focus", () => {
  const source = readFileSync(join(__dirname, "TerminalPanel.tsx"), "utf8");
  const subscriptionStart = source.indexOf(
    "const unsubscribe = window.codexDesktop.subscribeTerminalState",
  );
  const subscriptionEnd = source.indexOf(
    "const requestSeq = terminalStateRequestSeqRef.current.begin();",
    subscriptionStart,
  );
  const subscriptionSource = source.slice(subscriptionStart, subscriptionEnd);
  const statusEffectStart = source.indexOf(
    "terminal.options.cursorBlink = isInteractive(activeTab.status);",
  );
  const statusEffectEnd = source.indexOf(
    "  useEffect(() => {\n    queueMicrotask(applyPendingTerminalFocus);",
    statusEffectStart,
  );
  const statusEffectSource = source.slice(statusEffectStart, statusEffectEnd);

  assert.notEqual(subscriptionStart, -1);
  assert.notEqual(subscriptionEnd, -1);
  assert.match(subscriptionSource, /setState\(event\.state\)/);
  assert.match(
    subscriptionSource,
    /terminalRef\.current\?\.write\(decodeBase64\(event\.deltaBase64\)\)/,
  );
  assert.doesNotMatch(subscriptionSource, /requestTerminalViewportFocus/);
  assert.doesNotMatch(subscriptionSource, /terminal\.focus\(\)/);
  assert.notEqual(statusEffectStart, -1);
  assert.notEqual(statusEffectEnd, -1);
  assert.match(statusEffectSource, /writeTerminalStatusMarker/);
  assert.doesNotMatch(statusEffectSource, /requestTerminalViewportFocus/);
  assert.doesNotMatch(statusEffectSource, /terminal\.focus\(\)/);
});

test("TerminalPanel terminal tabs are concrete workspace drag sources", () => {
  const source = readFileSync(join(__dirname, "TerminalPanel.tsx"), "utf8");

  assert.match(source, /terminalTabDragPayload/);
  assert.match(source, /terminalTabId: tab\.id/);
  assert.match(source, /sessionId: tab\.sessionId/);
  assert.match(source, /threadId: tab\.threadId/);
  assert.match(source, /cwd: tab\.cwd/);
  assert.match(source, /commandItemId: tab\.commandItemId/);
  assert.match(source, /command: tab\.title/);
  assert.match(source, /draggable=\{onOpenTerminalTabInWorkspace != null\}/);
  assert.match(source, /writeWorkspaceObjectDragData\([\s\S]*terminalTabDragPayload\(tab\)/);
  assert.match(source, /activeTerminalTabId/);
  assert.match(source, /selectTerminalTab\(activeTerminalTabId\)/);
});

test("TerminalPanel workspace variant renders only the detached session content surface", () => {
  const source = readFileSync(join(__dirname, "TerminalPanel.tsx"), "utf8");
  const managerStart = source.indexOf("{isManagerVariant ? (");
  const viewportIndex = source.indexOf('className="terminal-viewport-shell"', managerStart);
  const managerSource = source.slice(managerStart, viewportIndex);

  assert.match(source, /variant = "manager"/);
  assert.match(source, /variant\?: "manager" \| "workspace"/);
  assert.match(
    source,
    /isManagerVariant \? "terminal-panel-manager" : "terminal-panel-workspace"/,
  );
  assert.notEqual(managerStart, -1);
  assert.notEqual(viewportIndex, -1);
  assert.match(managerSource, /panel-content-header terminal-header/);
  assert.match(managerSource, /aria-label="Live Commands"/);
  assert.match(managerSource, /aria-label="Terminal tabs"/);
  assert.match(managerSource, /aria-label="New shell terminal"/);
  assert.match(managerSource, /aria-label="Terminal display settings"/);
  assert.match(managerSource, /aria-label="Terminate active terminal"/);
  assert.match(source, /className="terminal-viewport-shell"/);
  assert.match(
    source,
    /isManagerVariant[\s\S]*Open a sandboxed shell or wait for a model PTY to become attachable\.[\s\S]*Terminal session is not available\./,
  );
}
);

test("TerminalPanel manager excludes the terminal session owned by workspace", () => {
  const source = readFileSync(join(__dirname, "TerminalPanel.tsx"), "utf8");

  assert.match(source, /detachedTerminalTabIds\?: string\[\]/);
  assert.match(source, /const detachedTerminalTabIdSet = useMemo\([\s\S]*new Set\(detachedTerminalTabIds\)/);
  assert.match(
    source,
    /const visibleTabs = useMemo\([\s\S]*state\.tabs\.filter\(\(tab\) => !detachedTerminalTabIdSet\.has\(tab\.id\)\)/,
  );
  assert.match(source, /\{visibleTabs\.map\(\(tab\) =>/);
  assert.match(
    source,
    /!visibleTabs\.some\(\(tab\) => tab\.id === focusTerminalTabRequest\.tabId\)/,
  );
  assert.match(source, /Terminal session is open in workspace\./);
});

test("TerminalPanel does not render a duplicate visible running status row below tabs", () => {
  const source = readFileSync(join(__dirname, "TerminalPanel.tsx"), "utf8");
  const tabStripIndex = source.indexOf('aria-label="Terminal tabs"');
  const viewportIndex = source.indexOf('className="terminal-viewport-shell"', tabStripIndex);
  const betweenTabsAndViewport = source.slice(tabStripIndex, viewportIndex);

  assert.notEqual(tabStripIndex, -1);
  assert.notEqual(viewportIndex, -1);
  assert.doesNotMatch(
    betweenTabsAndViewport,
    /\$\{activeTab\.status\}|\$\{activeTab\?\.status/,
  );
  assert.doesNotMatch(betweenTabsAndViewport, /No terminal tabs/);
  assert.match(betweenTabsAndViewport, /localError \|\| state\.error/);
});

import test from "node:test";
import assert from "node:assert/strict";
import {
  readSource,
  sourceIndex,
  sourceSlice,
} from "../test/sourceAssertions";

const terminalPanelSource = () => readSource(new URL("./TerminalPanel.tsx", import.meta.url));
const stylesSource = () => readSource(new URL("../styles.css", import.meta.url));

test("TerminalPanel header uses the compact single-title panel style", () => {
  const source = terminalPanelSource();

  assert.match(source, /className="panel-content-header terminal-header"/);
  assert.doesNotMatch(source, /panel-eyebrow/);
  assert.doesNotMatch(source, /<p title=\{activeTab\?\.cwd\}>/);
});

test("TerminalPanel attaches xterm input forwarding before replay writes", () => {
  const source = terminalPanelSource();
  const onDataIndex = sourceIndex(source, "dataSubscription = terminal.onData");
  const onBinaryIndex = sourceIndex(source, "binarySubscription = terminal.onBinary");
  const recordedSizeIndex = sourceIndex(source, "resizeToRecordedReplaySize();");
  const replayWriteIndex = sourceIndex(
    source,
    "terminal.write(decodeBase64(activeTab.replayBase64))",
  );
  const postReplayFitIndex = sourceIndex(
    source,
    "if (shouldReplayAtRecordedSize) {\n          sendSize();",
    replayWriteIndex,
  );

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
  const source = terminalPanelSource();
  sourceIndex(
    source,
    'Promise.all([import("@xterm/xterm"), import("@xterm/addon-fit")])',
  );
  const runtimeAddonImports = source.match(/import\("@xterm\/addon-fit"\)/g) ?? [];

  assert.equal(
    source.includes('import { FitAddon } from "@xterm/addon-fit"'),
    false,
  );
  assert.equal(runtimeAddonImports.length, 1);
  assert.match(source, /import type \{ FitAddon as XTermFitAddon \} from "@xterm\/addon-fit"/);
});

test("TerminalPanel publishes fitted size as thread preferred terminal size", () => {
  const source = terminalPanelSource();
  const fitIndex = sourceIndex(source, "const next = { rows: terminal.rows, cols: terminal.cols };");
  const preferredIndex = sourceIndex(source, "publishPreferredTerminalSize(next);");
  sourceIndex(source, ".resizeTerminal({ tabId: mountedTabId, size: next })");
  sourceIndex(source, "previousPreferred?.threadId !== threadId");

  assert.ok(
    fitIndex < preferredIndex,
    "preferred terminal size must come from the fitted xterm viewport",
  );
});

test("TerminalPanel converts LF-only output only for read-only command views", () => {
  const source = terminalPanelSource();
  const helperIndex = sourceIndex(source, "function shouldConvertTerminalEol");
  sourceIndex(source, "tab.readOnlyOutput === true", helperIndex);
  sourceIndex(source, "!tab.canWrite && !tab.canResize", helperIndex);
  const constructorIndex = sourceIndex(source, "terminal = new Terminal({");
  sourceIndex(
    source,
    "convertEol: shouldConvertTerminalEol(activeTab)",
    constructorIndex,
  );
});

test("TerminalPanel uses a light xterm surface theme", () => {
  const source = terminalPanelSource();
  const themeStart = sourceIndex(source, "const TERMINAL_THEME = {");
  const themeEnd = sourceIndex(source, "};", themeStart);
  const constructorIndex = sourceIndex(source, "terminal = new Terminal({");
  sourceIndex(source, "theme: TERMINAL_THEME", constructorIndex);
  const themeSource = source.slice(themeStart, themeEnd);

  assert.match(themeSource, /background: "#fbfaf8"/);
  assert.match(themeSource, /foreground: "#292524"/);
  assert.doesNotMatch(themeSource, /#111827/);
});

test("TerminalPanel viewport chrome uses the light panel surface", () => {
  const source = stylesSource();
  const shellStart = sourceIndex(source, ".terminal-viewport-shell {");
  const emptyStart = sourceIndex(source, ".terminal-empty {", shellStart);
  const emptyButtonEnd = sourceIndex(source, ".preview-editor-shell {", emptyStart);
  const terminalViewportSource = source.slice(shellStart, emptyButtonEnd);

  assert.match(terminalViewportSource, /background: #fbfaf8/);
  assert.match(terminalViewportSource, /background: #f5f3f0/);
  assert.match(terminalViewportSource, /scrollbar-color: #d6d3d1 #fbfaf8/);
  assert.doesNotMatch(terminalViewportSource, /#111827/);
});

test("TerminalPanel rebuilds xterm when fallback upgrades to live capabilities", () => {
  const dependencies = sourceSlice(
    terminalPanelSource(),
    "  }, [\n    activeTab?.id,",
    "  ]);",
  );

  assert.match(dependencies, /activeTab\?\.canResize/);
  assert.match(dependencies, /activeTab\?\.canWrite/);
  assert.match(dependencies, /activeTab\?\.readOnlyOutput/);
});

test("TerminalPanel rebuilds xterm when a focused command receives replay output", () => {
  const dependencies = sourceSlice(
    terminalPanelSource(),
    "  }, [\n    activeTab?.id,",
    "  ]);",
  );

  assert.match(dependencies, /activeTab\?\.replayBase64/);
  assert.match(dependencies, /activeTab\?\.replayTruncated/);
  assert.match(dependencies, /activeTab\?\.hasSequenceGap/);
});

test("TerminalPanel does not rebuild xterm for streaming output deltas", () => {
  const source = terminalPanelSource();
  const dependencies = sourceSlice(source, "  }, [\n    activeTab?.id,", "  ]);");
  sourceIndex(
    source,
    "terminalRef.current?.write(decodeBase64(event.deltaBase64));",
  );

  assert.doesNotMatch(dependencies, /activeTab\?\.replayThroughSequence/);
  assert.doesNotMatch(dependencies, /activeTab\?\.status/);
});

test("TerminalPanel input and resize handlers read the latest active tab runtime", () => {
  const source = terminalPanelSource();
  sourceIndex(source, "const activeTabRuntimeRef = useRef");
  sourceIndex(source, "activeTabRuntimeRef.current = activeTab");
  const sendSizeIndex = sourceIndex(source, "sendSize = () => {");
  sourceIndex(
    source,
    "const currentTab = activeTabRuntimeRef.current;",
    sendSizeIndex,
  );
  sourceIndex(
    source,
    "currentTab?.id === mountedTabId",
    sendSizeIndex,
  );
  const dataIndex = sourceIndex(source, "dataSubscription = terminal.onData");
  sourceIndex(
    source,
    "const currentTab = activeTabRuntimeRef.current;",
    dataIndex,
  );
  sourceIndex(
    source,
    "currentTab?.id !== mountedTabId",
    dataIndex,
  );
  const binaryIndex = sourceIndex(source, "binarySubscription = terminal.onBinary");
  sourceIndex(
    source,
    "const currentTab = activeTabRuntimeRef.current;",
    binaryIndex,
  );
  sourceIndex(
    source,
    "currentTab?.id !== mountedTabId",
    binaryIndex,
  );
});

test("TerminalPanel writes terminal exit markers without remounting xterm", () => {
  const source = terminalPanelSource();
  const helperIndex = sourceIndex(source, "function writeTerminalStatusMarker");
  const mountMarkerIndex = sourceIndex(
    source,
    "writeTerminalStatusMarker(terminal, activeTab, terminalStatusMarkerRef);",
  );
  sourceIndex(
    source,
    "terminal.options.cursorBlink = isInteractive(activeTab.status);",
  );
  sourceIndex(
    source,
    "[Session disconnected from the runtime.]",
    helperIndex,
  );
  sourceIndex(source, "[Process exited", helperIndex);
  sourceIndex(
    source,
    "terminalStatusMarkerRef.current = null;",
    sourceIndex(source, "return () => {", mountMarkerIndex),
  );
});

test("TerminalPanel publishes preferred size while idle with no active tab", () => {
  const source = terminalPanelSource();
  const idleEffectIndex = sourceIndex(
    source,
    "if (activeTab) {\n      return undefined;",
  );
  const measureIndex = sourceIndex(
    source,
    "measureTerminalViewportSize(viewport, displayPreferences)",
  );
  sourceIndex(
    source,
    "publishPreferredTerminalSize(next);",
    measureIndex,
  );
  const viewportIndex = sourceIndex(
    source,
    "className={`terminal-viewport ${activeTab ? \"\" : \"idle\"}`}",
  );
  const emptyIndex = sourceIndex(source, "{!activeTab ? (");

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
  const source = terminalPanelSource();
  sourceIndex(
    source,
    "const requestTerminalViewportFocus = useCallback",
  );
  const applyHelperIndex = sourceIndex(source, "const applyPendingTerminalFocus = useCallback");
  sourceIndex(
    source,
    "lastAppliedTerminalFocusTokenRef.current = request.token;\n    terminal.focus();",
    applyHelperIndex,
  );
  sourceIndex(
    source,
    "shouldApplyTerminalViewportFocusRequest({",
    applyHelperIndex,
  );
  sourceIndex(source, "applyPendingTerminalFocus();");
  const unconditionalMountFocusIndex = source.indexOf("terminal?.focus();");
  sourceIndex(
    source,
    "requestTerminalViewportFocus(nextState.activeTabId);",
  );
  sourceIndex(
    source,
    "requestTerminalViewportFocus(nextState.activeTabId);",
    sourceIndex(source, "const createTerminal = () => {"),
  );
  const liveCommandFocusIndex = sourceIndex(
    source,
    "const focusLiveCommand = (command: (typeof liveCommands)[number]) => {",
  );
  sourceIndex(
    source,
    "requestTerminalViewportFocus(tab.id);",
    sourceIndex(source, ".selectTerminalTab(tab.id)"),
  );

  assert.equal(
    unconditionalMountFocusIndex,
    -1,
    "terminal mount/replay updates must not unconditionally steal focus",
  );
  sourceIndex(
    source,
    "requestTerminalViewportFocus(nextState.activeTabId);",
    liveCommandFocusIndex,
  );
});

test("TerminalPanel passive state refreshes do not request xterm focus", () => {
  const source = terminalPanelSource();
  const subscriptionStart = sourceIndex(
    source,
    "const unsubscribe = window.codexDesktop.subscribeTerminalState",
  );
  const subscriptionEnd = sourceIndex(
    source,
    "const requestSeq = terminalStateRequestSeqRef.current.begin();",
    subscriptionStart,
  );
  const subscriptionSource = source.slice(subscriptionStart, subscriptionEnd);
  const statusEffectStart = sourceIndex(
    source,
    "terminal.options.cursorBlink = isInteractive(activeTab.status);",
  );
  const statusEffectEnd = sourceIndex(
    source,
    "  useEffect(() => {\n    queueMicrotask(applyPendingTerminalFocus);",
    statusEffectStart,
  );
  const statusEffectSource = source.slice(statusEffectStart, statusEffectEnd);

  assert.match(subscriptionSource, /setState\(event\.state\)/);
  assert.match(
    subscriptionSource,
    /terminalRef\.current\?\.write\(decodeBase64\(event\.deltaBase64\)\)/,
  );
  assert.doesNotMatch(subscriptionSource, /requestTerminalViewportFocus/);
  assert.doesNotMatch(subscriptionSource, /terminal\.focus\(\)/);
  assert.match(statusEffectSource, /writeTerminalStatusMarker/);
  assert.doesNotMatch(statusEffectSource, /requestTerminalViewportFocus/);
  assert.doesNotMatch(statusEffectSource, /terminal\.focus\(\)/);
});

test("TerminalPanel terminal tabs are concrete workspace drag sources", () => {
  const source = terminalPanelSource();

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
  const source = terminalPanelSource();
  const managerStart = sourceIndex(source, "{isManagerVariant ? (");
  const viewportIndex = sourceIndex(source, 'className="terminal-viewport-shell"', managerStart);
  const managerSource = source.slice(managerStart, viewportIndex);

  assert.match(source, /variant = "manager"/);
  assert.match(source, /variant\?: "manager" \| "workspace"/);
  assert.match(
    source,
    /isManagerVariant \? "terminal-panel-manager" : "terminal-panel-workspace"/,
  );
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
  const source = terminalPanelSource();

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
  const source = terminalPanelSource();
  const tabStripIndex = sourceIndex(source, 'aria-label="Terminal tabs"');
  const viewportIndex = sourceIndex(source, 'className="terminal-viewport-shell"', tabStripIndex);
  const betweenTabsAndViewport = source.slice(tabStripIndex, viewportIndex);

  assert.doesNotMatch(
    betweenTabsAndViewport,
    /\$\{activeTab\.status\}|\$\{activeTab\?\.status/,
  );
  assert.doesNotMatch(betweenTabsAndViewport, /No terminal tabs/);
  assert.match(betweenTabsAndViewport, /localError \|\| state\.error/);
});

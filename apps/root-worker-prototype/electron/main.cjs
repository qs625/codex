const path = require("node:path");
const fs = require("node:fs/promises");
const http = require("node:http");
const { randomUUID } = require("node:crypto");
const { pathToFileURL } = require("node:url");
const {
  app,
  BrowserWindow,
  dialog,
  ipcMain,
  net,
  Notification,
  protocol,
  session,
  shell,
  systemPreferences,
  webContents: electronWebContents,
  WebContentsView,
} = require("electron");
const { AppServerClient } = require("./appServerClient.cjs");
const {
  browserNavigationEventDecision,
  browserNavigationEventTarget,
  normalizeBrowserDebugTarget,
  normalizeBrowserTarget,
} = require("./browserPanelSecurity.cjs");
const {
  browserPanelWebPreferences,
  browserSessionPartition,
} = require("./browserPanelConfig.cjs");
const {
  normalizeBrowserBoundsUpdate,
} = require("./browserPanelBounds.cjs");
const {
  allowBrowserPanelPermission,
  allowDefaultSessionPermission,
  configurePermissionHandlers,
} = require("./permissionHandlers.cjs");
const {
  nextBrowserTabIdAfterClose,
  shouldDetachAttachedBrowserPanelView,
} = require("./browserPanelTabs.cjs");
const {
  isLocalLinkTarget,
  localFilePathFromTarget,
  parseLocalFileTarget,
} = require("./fileTargets.cjs");
const {
  buildPdfPreview,
  FILE_PREVIEW_PROTOCOL,
} = require("./localFilePreview.cjs");
const { writeLocalFileTarget } = require("./localFileWrite.cjs");
const { languageForFilePath } = require("./filePreviewLanguages.cjs");
const {
  readGitCommitFiles,
  readGitFileDiff,
  readGitSnapshot,
  readGitStatusSnapshot,
} = require("./gitPanel.cjs");
const { LspManager } = require("./lsp/manager.cjs");
const {
  normalizeThreadLifecycleStatus,
} = require("./threadLifecycleStatus.cjs");
const { normalizeThreadSnapshot } = require("./threadSnapshots.cjs");
const {
  buildChatCompatCwd,
  buildCreateThreadStartParams,
} = require("./threadConfig.cjs");
const { listThreads: listAllThreads } = require("./threadList.cjs");
const {
  ensureSelfProjectSync,
  recordSystemSelfThreadIdSync,
} = require("./selfProject.cjs");
const {
  ensureSelfProjectThread,
  sendSelfCommandToThread,
} = require("./selfProjectThread.cjs");
const { buildTurnInput } = require("./turnInput.cjs");
const {
  buildTurnStartParams,
  mergeRuntimeOverride,
  resolveRuntimeForResume,
} = require("./turnStart.cjs");
const {
  ensureDefaultWorkspace,
  isPackagedApp,
  resolveDefaultWorkspace,
} = require("./workspace.cjs");
const {
  showSystemNotification,
} = require("./systemNotification.cjs");
const {
  createAppRelaunchAdapter,
  createClientRelaunchNotificationHandler,
  createInstalledArtifactUpdateLifecycleAdapter,
  createRendererReloadLifecycleAdapter,
  isClientRelaunchNotification,
  observeClientRelaunchResult,
} = require("./appLifecycle.cjs");
const {
  removeInstalledArtifactTree,
  resolveInstalledArtifactUpdatePlanInWorker,
  updateInstalledArtifactsInWorker,
} = require("./installedArtifactUpdate.cjs");
const {
  createJsonAutoResumeStateStore,
  createThreadAutoResumeCoordinator,
} = require("./threadAutoResume.cjs");
const {
  createRuntimeRestartController,
  createRuntimeRestartIntentStore,
  recoverRuntimeRestartAfterThreadTerminal,
} = require("./runtimeRestartIntent.cjs");
const {
  expectedRuntimeRestartRecoveryPrompt,
  formatPayloadRuntimeRecoveryPrompt,
  shouldNotifyRuntimeRestartRecoveryOnSelf,
} = require("./restartRecoveryPrompts.cjs");
const {
  notifyRecoverableRestartErrorOnSelf,
} = require("./restartRecoverySelfNotice.cjs");
const { createRuntimeLauncher } = require("./runtimeLauncher.cjs");
const {
  recoverLauncherStateAtStartup,
  recoverPayloadRuntimeFailureIfPresent,
  recordLauncherRecoveryIfPresent,
  writePayloadFailureEvidence,
} = require("./runtimeLaunchState.cjs");
const { applyRemoteDebuggingConfig } = require("./remoteDebugging.cjs");
const {
  startRemoteDebuggingProxy,
} = require("./remoteDebuggingProxy.cjs");
const {
  activeCommandForTerminalFocus,
  addUserTerminal,
  appendCommandOutputCache,
  appendTerminalOutput,
  applyCommandOutputCacheToDescriptor,
  closeTerminalTab,
  commandFocusDescriptorForTerminalFocus,
  deleteCommandOutputCache,
  focusCommandTerminal,
  createTerminalPanelState,
  liveCommandSessionForTerminalFocus,
  markRunningTerminalsLost,
  markTerminalExited,
  mergeTerminalSessions,
  reattachTerminalSessions,
  isTerminalSessionDetached,
  setTerminalTabSize,
  selectTerminalTab,
  terminalPanelSnapshot,
  terminalTabMetadata,
  terminalTabNeedsLiveSessionRefresh,
  terminalTabSupports,
} = require("./terminalPanel.cjs");

const rendererMode = process.env.ROOT_WORKER_RENDERER_MODE ?? "built";
const isDev = rendererMode === "dev";
const appServerClient = new AppServerClient();
const lspManager = new LspManager();
const runtimeLauncher = createRuntimeLauncher();
const appRelaunch = createAppRelaunchAdapter({
  app,
  beforeExit: (reason) =>
    appServerClient.stop(reason ?? "application relaunch"),
});
const rendererReloadLifecycle = createRendererReloadLifecycleAdapter({
  fullRelaunch: appRelaunch,
  reloadWindows: reloadRendererWindows,
  broadcastStatus: (status) =>
    broadcast("codex:status", {
      ...appServerClient.status,
      ...status,
  }),
  logger: console,
});
const installedArtifactUpdateLifecycle =
  createInstalledArtifactUpdateLifecycleAdapter({
    appExit: (code) => app.exit(code),
    cleanupPreparedArtifact: (preparedRoot) =>
      removeInstalledArtifactTree(preparedRoot),
    resolvePlan: () => resolveInstalledArtifactUpdatePlanInWorker(),
    runtimeLauncher,
    updateArtifacts: (plan) => updateInstalledArtifactsInWorker(plan),
    gracefulShutdownAppServer: (reason) =>
      appServerClient.gracefulShutdown(reason ?? "Runtime Capsule switch"),
    broadcastStatus: (status) =>
      broadcast("codex:status", {
        ...appServerClient.status,
        ...status,
      }),
    logger: console,
  });
const handleClientRelaunchNotification =
  createClientRelaunchNotificationHandler({
    rendererReload: rendererReloadLifecycle,
    installedArtifactUpdate: installedArtifactUpdateLifecycle,
    fullRelaunch: appRelaunch,
  });
const windows = new Set();
const browserPanelsByWindowId = new Map();
const terminalPanelsByWindowId = new Map();
const commandOutputCache = new Map();
const MAX_PENDING_TERMINAL_NOTIFICATIONS = 4096;
let browserPanelTabCounter = 0;
const threadRuntimeById = new Map();
const localFilePreviewTargetsByToken = new Map();
let autoResumeCoordinator = null;
let runtimeRestartController = null;
let runtimeRestartIntentStore = null;
let quittingAfterAppServerStop = false;
let fatalPayloadExitRequested = false;
let startupRuntimeRecovery = { hasDurableRestartRecovery: false };
const defaultWorkspace = resolveDefaultWorkspace();
const devServerUrl =
  process.env.ROOT_WORKER_DEV_SERVER_URL ?? "http://127.0.0.1:5173";

const remoteDebuggingConfig = applyRemoteDebuggingConfig(app, process.env, console);
let remoteDebuggingProxy = null;

process.on("uncaughtException", requestFatalPayloadExit);
process.on("unhandledRejection", requestFatalPayloadExit);

protocol.registerSchemesAsPrivileged([
  {
    scheme: FILE_PREVIEW_PROTOCOL,
    privileges: {
      standard: true,
      secure: true,
      supportFetchAPI: true,
      stream: true,
    },
  },
]);

async function createWindow() {
  const window = new BrowserWindow({
    width: 1520,
    height: 980,
    minWidth: 1280,
    minHeight: 820,
    title: "Root Worker Prototype",
    backgroundColor: "#0c1117",
    webPreferences: {
      contextIsolation: true,
      nodeIntegration: false,
      preload: path.join(__dirname, "preload.cjs"),
    },
  });

  windows.add(window);
  window.on("closed", () => {
    destroyBrowserPanel(window);
    terminalPanelsByWindowId.delete(window.id);
    windows.delete(window);
  });

  if (isDev) {
    await window.loadURL(devServerUrl);
    if (process.env.ROOT_WORKER_OPEN_DEVTOOLS !== "0") {
      window.webContents.openDevTools({ mode: "detach" });
    }
  } else {
    await ensureBuiltRenderer();
    await window.loadFile(builtRendererPath());
  }
  return window;
}

async function primeMicrophoneAccessPrompt() {
  if (process.platform !== "darwin") {
    return;
  }
  const before = systemPreferences.getMediaAccessStatus("microphone");
  if (before !== "not-determined") {
    return;
  }
  try {
    await systemPreferences.askForMediaAccess("microphone");
  } catch (error) {
    console.error(
      "[prototype] microphone access prime failed",
      JSON.stringify({
        message: error instanceof Error ? error.message : String(error),
      }),
    );
  }
}

function broadcast(channel, payload) {
  for (const window of windows) {
    if (!window.isDestroyed()) {
      window.webContents.send(channel, payload);
    }
  }
}

appServerClient.on("notification", (notification) => {
  if (notification.method === "thread/started") {
    const thread = notification.params?.thread ?? null;
    console.error(
      "[prototype] thread/started",
      JSON.stringify({
        threadId: thread?.id ?? null,
        threadSource: thread?.threadSource ?? null,
        source: thread?.source ?? null,
      }),
    );
  }
  const normalizedNotification = normalizeNotification(notification);
  routeTerminalNotification(normalizedNotification);
  // The started event carries a server-generated resume capability. Keep it
  // in Electron main-process state rather than exposing it to renderer IPC.
  if (normalizedNotification.method === "command/exec/started") {
    return;
  }
  if (isClientRelaunchNotification(normalizedNotification)) {
    void getRuntimeRestartController().handle(normalizedNotification);
    return;
  }
  if (
    normalizedNotification.method === "thread/status/changed" &&
    normalizedNotification.params?.lifecycleStatus?.type === "final"
  ) {
    void recoverRuntimeRestartAfterThreadTerminal(
      getRuntimeRestartController(),
      normalizedNotification,
    ).catch((error) => {
      console.error(
        "[prototype] runtime restart recovery after thread terminal failed",
        error,
      );
    });
  }
  broadcast("codex:notification", normalizedNotification);
});

appServerClient.on("request", (request) => {
  if (!isApprovalServerRequest(request)) {
    void appServerClient.rejectServerRequest(
      request.id,
      `Unsupported server request from app-server: ${request.method}`,
    );
    return;
  }
  if (windows.size === 0) {
    void appServerClient.rejectServerRequest(
      request.id,
      `No renderer available for app-server request: ${request.method}`,
    );
    return;
  }
  broadcast("codex:request", normalizeRequest(request));
});

appServerClient.on("status", (status) => {
  if (!status.connected) {
    for (const panel of terminalPanelsByWindowId.values()) {
      markRunningTerminalsLost(panel.state);
      sendTerminalPanelState(panel);
    }
  } else {
    for (const panel of terminalPanelsByWindowId.values()) {
      void refreshTerminalPanelSessions(panel);
    }
  }
  broadcast("codex:status", status);
});

ipcMain.handle("codex:health", async () => {
  await ensureDefaultWorkspace();
  await appServerClient.ready();
  return {
    ok: true,
    appServer: appServerClient.status,
    workspace: defaultWorkspace,
  };
});

ipcMain.handle("codex:showSystemNotification", async (_event, payload) =>
  showSystemNotification(payload, { Notification }),
);

ipcMain.handle("codex:relaunchApp", async (_event, payload = {}) => {
  return requestClientRelaunch(payload?.reason ?? "renderer");
});

ipcMain.handle("codex:bootstrap", async () => {
  await ensureDefaultWorkspace();
  const initialListResult = await listThreads(defaultWorkspace);
  const initialThreads = initialListResult.threads;
  const expectedRestart =
    await getRuntimeRestartController().recoverPending();
  const autoResume =
    await getAutoResumeCoordinator().runAfterRuntimeRestartRecovery({
      hasDurableRestartRecovery:
        startupRuntimeRecovery.hasDurableRestartRecovery,
      recoveryOccurrenceId:
        expectedRestart.recoveryOccurrenceId ??
        startupRuntimeRecovery.recoveryOccurrenceId,
      threads: initialThreads,
      expectedRestart,
    });
  const listResult = await listThreads(defaultWorkspace);
  return {
    workspace: defaultWorkspace,
    threads: listResult.threads,
    materializedSelfThreadId: listResult.materializedSelfThreadId,
    autoResume,
    expectedRestart,
    appServer: appServerClient.status,
  };
});

ipcMain.handle("codex:listThreads", async (_event, cwd = defaultWorkspace) => {
  await ensureDefaultWorkspace();
  return { data: (await listThreads(cwd)).threads };
});

ipcMain.handle("codex:listModels", async () => {
  await ensureDefaultWorkspace();
  return appServerClient.request("model/list", { includeHidden: false });
});

ipcMain.handle("codex:readConfig", async (_event, payload = {}) => {
  await ensureDefaultWorkspace();
  return appServerClient.request("config/read", {
    includeLayers: Boolean(payload?.includeLayers),
    cwd: payload?.cwd ?? null,
  });
});

ipcMain.handle("codex:writeConfigValue", async (_event, payload) => {
  await ensureDefaultWorkspace();
  return appServerClient.request("config/value/write", payload);
});

ipcMain.handle("codex:batchWriteConfig", async (_event, payload) => {
  await ensureDefaultWorkspace();
  return appServerClient.request("config/batchWrite", payload);
});

ipcMain.handle("codex:readAccount", async (_event, payload = {}) => {
  await ensureDefaultWorkspace();
  return appServerClient.request("account/read", {
    refreshToken: Boolean(payload?.refreshToken),
  });
});

ipcMain.handle("codex:androidConnectionInfo", async () => {
  await ensureDefaultWorkspace();
  await appServerClient.ready();
  return appServerClient.getMobileConnectionInfo();
});

ipcMain.handle("codex:startAccountLogin", async (_event, payload) => {
  await ensureDefaultWorkspace();
  return appServerClient.request("account/login/start", payload);
});

ipcMain.handle("codex:cancelAccountLogin", async (_event, payload) => {
  await ensureDefaultWorkspace();
  return appServerClient.request("account/login/cancel", payload);
});

ipcMain.handle("codex:listAgentTypes", async (_event, cwd = defaultWorkspace) => {
  await ensureDefaultWorkspace();
  return appServerClient.request("agentType/list", { cwd });
});

ipcMain.handle("codex:listThreadProviders", async (_event, cwd = defaultWorkspace) => {
  await ensureDefaultWorkspace();
  return appServerClient.request("threadProvider/list", { cwd });
});

ipcMain.handle("codex:selectProjectDirectory", async (event, defaultPath) => {
  const window = BrowserWindow.fromWebContents(event.sender);
  const result = await dialog.showOpenDialog(window ?? undefined, {
    defaultPath: typeof defaultPath === "string" ? defaultPath : defaultWorkspace,
    properties: ["openDirectory"],
  });
  if (result.canceled || result.filePaths.length === 0) {
    return { path: null };
  }
  return { path: result.filePaths[0] };
});

ipcMain.handle("codex:listSkills", async (_event, cwd = defaultWorkspace) => {
  await ensureDefaultWorkspace();
  return listSkills(cwd);
});

ipcMain.handle("codex:listWorkflows", async (_event, cwd = defaultWorkspace) => {
  await ensureDefaultWorkspace();
  return appServerClient.request("workflow/list", { cwd });
});

ipcMain.handle("codex:createThread", async (_event, payload) => {
  await ensureDefaultWorkspace();
  const chatCompatCwd = buildChatCompatCwd(app.getPath("userData"));
  if (payload?.threadMode === "chat") {
    await fs.mkdir(chatCompatCwd, { recursive: true });
  }
  const params = buildCreateThreadStartParams(payload, { chatCompatCwd });
  const start = await appServerClient.request("thread/start", params);

  const name = payload?.name?.trim();
  if (payload?.name && payload.name.trim()) {
    await appServerClient.request("thread/name/set", {
      threadId: start.thread.id,
      name,
    });
  }

  const runtime = {
    model: start.model ?? null,
    modelProvider: start.modelProvider ?? null,
    reasoningEffort: start.reasoningEffort ?? null,
  };
  rememberThreadRuntime(start.thread.id, runtime);
  return {
    thread: normalizeThread(
      name ? { ...start.thread, name } : start.thread,
      runtime,
    ),
  };
});

ipcMain.handle("codex:getSelfProject", async () => {
  const project = await ensureSelfProjectForCurrentApp();
  return { project };
});

ipcMain.handle("codex:startSelfCommand", async (_event, payload = {}) => {
  const project = await ensureSelfProjectForCurrentApp();
  if (!project) {
    throw new Error("Self project is only available from a packaged app.");
  }
  const result = await sendSelfCommandToThread({
    appServerClient,
    buildTurnInput,
    loadThreadForTurn: async (threadId) =>
      (await subscribeThread(threadId)).thread ?? null,
    normalizeThread,
    persistSystemThreadId: persistCurrentSystemSelfThreadId,
    project,
    rememberThreadRuntime,
    startThreadTurn,
    text: payload?.text,
    threads: await listAllThreads(appServerClient, normalizeThread),
  });
  return {
    project,
    materializedSelfThreadId: result.materializedSelfThreadId,
    thread: result.thread,
    turn: result.turn,
  };
});

ipcMain.handle("codex:archiveThread", async (_event, threadId) => {
  await appServerClient.request("thread/archive", { threadId });
  threadRuntimeById.delete(threadId);
  return { ok: true };
});

ipcMain.handle(
  "codex:readThread",
  async (_event, threadId, includeTurns = true) => {
    return readThread(
      threadId,
      includeTurns,
      threadRuntimeById.get(threadId) ?? null,
    );
  },
);

ipcMain.handle("codex:readCompactHistory", async (_event, threadId) => {
  return readThread(
    threadId,
    true,
    threadRuntimeById.get(threadId) ?? null,
    { includeCompactReplacementHistory: true },
  );
});

ipcMain.handle("codex:setThreadRunConfig", async (_event, payload) => {
  rememberThreadRuntime(payload.threadId, {
    model: payload.model ?? null,
    modelProvider: payload.modelProvider ?? null,
    reasoningEffort: payload.reasoningEffort ?? null,
    localOverride: true,
  });
  return { ok: true };
});

ipcMain.handle("codex:subscribeThread", async (_event, threadId) => {
  return subscribeThread(threadId);
});

ipcMain.handle("codex:unsubscribeThread", async (_event, threadId) => {
  return unsubscribeThread(threadId);
});

ipcMain.handle("codex:getThreadGoal", async (_event, threadId) => {
  const response = await appServerClient.request("thread/goal/get", {
    threadId,
  });
  return {
    goal: response.goal ? normalizeThreadGoal(response.goal) : null,
  };
});

ipcMain.handle("codex:setThreadGoal", async (_event, payload) => {
  const response = await appServerClient.request("thread/goal/set", {
    threadId: payload.threadId,
    objective: payload.objective,
    status: payload.status,
  });
  return {
    goal: normalizeThreadGoal(response.goal),
  };
});

ipcMain.handle("codex:clearThreadGoal", async (_event, threadId) => {
  return appServerClient.request("thread/goal/clear", {
    threadId,
  });
});

ipcMain.handle("codex:openLink", async (_event, target) => {
  await openLinkTarget(target);
  return { ok: true };
});

ipcMain.handle("codex:browser:show", async (event, bounds) => {
  const panel = browserPanelForEvent(event);
  attachBrowserPanel(panel);
  setBrowserPanelBounds(panel, bounds);
  return browserPanelState(panel);
});

ipcMain.handle("codex:browser:hide", async (event) => {
  const panel = browserPanelForEvent(event);
  detachBrowserPanel(panel);
  return browserPanelState(panel);
});

ipcMain.handle("codex:browser:setBounds", async (event, bounds) => {
  const panel = browserPanelForEvent(event);
  setBrowserPanelBounds(panel, bounds);
  return browserPanelState(panel);
});

ipcMain.handle("codex:browser:navigate", async (event, target) => {
  const panel = browserPanelForEvent(event);
  const tab = activeBrowserPanelTab(panel);
  if (!tab) {
    throw new Error("Browser panel has no active tab");
  }
  const normalized = normalizeBrowserTarget(target);
  if (!normalized.ok) {
    throw new Error(normalized.reason);
  }
  tab.state.error = null;
  await loadBrowserPanelTabUrl(panel, tab, normalized.url);
  return browserPanelState(panel);
});

ipcMain.handle("codex:browser:newTab", async (event, target) => {
  const panel = browserPanelForEvent(event);
  const normalized =
    typeof target === "string" && target.trim()
      ? normalizeBrowserTarget(target)
      : { ok: true, url: null };
  if (!normalized.ok) {
    throw new Error(normalized.reason);
  }
  const tab = createBrowserPanelTab(panel, { activate: true });
  if (normalized.url) {
    await loadBrowserPanelTabUrl(panel, tab, normalized.url);
  }
  return browserPanelState(panel);
});

ipcMain.handle("codex:browser:selectTab", async (event, tabId) => {
  const panel = browserPanelForEvent(event);
  if (!selectBrowserPanelTab(panel, tabId)) {
    throw new Error("Browser tab not found");
  }
  return browserPanelState(panel);
});

ipcMain.handle("codex:browser:closeTab", async (event, tabId) => {
  const panel = browserPanelForEvent(event);
  if (!closeBrowserPanelTab(panel, tabId)) {
    throw new Error("Browser tab not found");
  }
  return browserPanelState(panel);
});

ipcMain.handle("codex:browser:goBack", async (event) => {
  const panel = browserPanelForEvent(event);
  const tab = activeBrowserPanelTab(panel);
  if (tab) {
    const navigation = browserNavigation(tab.view.webContents);
    if (navigation.canGoBack()) {
      navigation.goBack();
    }
  }
  return browserPanelState(panel);
});

ipcMain.handle("codex:browser:goForward", async (event) => {
  const panel = browserPanelForEvent(event);
  const tab = activeBrowserPanelTab(panel);
  if (tab) {
    const navigation = browserNavigation(tab.view.webContents);
    if (navigation.canGoForward()) {
      navigation.goForward();
    }
  }
  return browserPanelState(panel);
});

ipcMain.handle("codex:browser:reload", async (event) => {
  const panel = browserPanelForEvent(event);
  const tab = activeBrowserPanelTab(panel);
  if (tab && tab.view.webContents.getURL()) {
    tab.view.webContents.reload();
  }
  return browserPanelState(panel);
});

ipcMain.handle("codex:browser:stop", async (event) => {
  const panel = browserPanelForEvent(event);
  const tab = activeBrowserPanelTab(panel);
  if (tab) {
    tab.view.webContents.stop();
    tab.state.loading = false;
  }
  sendBrowserPanelState(panel);
  return browserPanelState(panel);
});

ipcMain.handle("codex:terminal:getState", async (event, threadId) => {
  const panel = terminalPanelForEvent(event);
  panel.threadId = threadId || null;
  await refreshTerminalPanelSessions(panel, panel.threadId);
  return terminalPanelState(panel);
});

ipcMain.handle("codex:terminal:create", async (event, payload = {}) => {
  const panel = terminalPanelForEvent(event);
  const processId = randomUUID();
  const command = [resolveInteractiveShell()];
  const descriptor = {
    sessionId: `user:${processId}`,
    generation: processId,
    origin: "user",
    threadId: null,
    commandItemId: null,
    processId,
    title: path.basename(command[0]) || "Shell",
    cwd: payload.cwd || defaultWorkspace,
    replayBase64: null,
    replayTruncated: false,
    replayThroughSequence: 0,
    size: normalizeTerminalSize(payload.size),
    canResize: true,
    canWrite: true,
    canTerminate: true,
  };
  const tab = addUserTerminal(panel.state, descriptor);
  panel.error = null;
  sendTerminalPanelState(panel);
  void appServerClient
    .request("command/exec", {
      command,
      processId,
      tty: true,
      disableTimeout: true,
      disableOutputCap: true,
      cwd: descriptor.cwd,
      size: normalizeTerminalSize(payload.size),
    })
    .catch((error) => {
      tab.status = "lost";
      tab.error = error instanceof Error ? error.message : String(error);
      sendTerminalPanelState(panel);
    });
  return terminalPanelState(panel);
});

ipcMain.handle("codex:terminal:select", async (event, tabId) => {
  const panel = terminalPanelForEvent(event);
  if (!selectTerminalTab(panel.state, tabId)) {
    throw new Error("Terminal tab not found");
  }
  sendTerminalPanelState(panel);
  return terminalPanelState(panel);
});

ipcMain.handle("codex:terminal:focusCommand", async (event, command) => {
  const panel = terminalPanelForEvent(event);
  if (
    !command ||
    typeof command.threadId !== "string" ||
    typeof command.commandItemId !== "string" ||
    (command.processId !== undefined &&
      command.processId !== null &&
      typeof command.processId !== "string")
  ) {
    throw new Error("Live command is no longer available");
  }
  const { thread } = await readThread(command.threadId, true);
  const activeCommand = activeCommandForTerminalFocus(thread, command);
  panel.threadId = command.threadId;
  const refreshed = await refreshTerminalPanelSessions(panel, command.threadId);
  const liveSession = refreshed
    ? liveCommandSessionForTerminalFocus(panel.state, command)
    : null;
  const focusDescriptor = commandFocusDescriptorForTerminalFocus(
    command,
    activeCommand,
    liveSession,
    { liveSessionRefreshed: refreshed },
  );
  if (!focusDescriptor) {
    throw new Error("Live command is no longer available");
  }
  const tab = focusCommandTerminal(
    panel.state,
    applyCommandOutputCacheToDescriptor(panel.state, focusDescriptor),
  );
  panel.error = null;
  sendTerminalPanelState(panel);
  return { state: terminalPanelState(panel), tabId: tab.id };
});

ipcMain.handle("codex:terminal:close", async (event, tabId) => {
  const panel = terminalPanelForEvent(event);
  if (!closeTerminalTab(panel.state, tabId)) {
    throw new Error("Terminal tab not found");
  }
  sendTerminalPanelState(panel);
  return terminalPanelState(panel);
});

ipcMain.handle("codex:terminal:reattach", async (event) => {
  const panel = terminalPanelForEvent(event);
  reattachTerminalSessions(panel.state);
  await refreshTerminalPanelSessions(panel);
  return terminalPanelState(panel);
});

ipcMain.handle("codex:terminal:write", async (event, payload) => {
  const panel = terminalPanelForEvent(event);
  const tab = requireTerminalTab(panel, payload.tabId);
  if (!terminalTabSupports(tab, "write")) {
    throw new Error("Terminal input is not supported by this execution environment");
  }
  await appServerClient.request("terminal/session/write", {
    ...terminalControlTarget(panel, tab),
    deltaBase64: payload.deltaBase64,
  });
  return { ok: true };
});

ipcMain.handle("codex:terminal:resize", async (event, payload) => {
  const panel = terminalPanelForEvent(event);
  const tab = requireTerminalTab(panel, payload.tabId);
  if (!terminalTabSupports(tab, "resize")) {
    throw new Error("Terminal resize is not supported by this execution environment");
  }
  const size = normalizeTerminalSize(payload.size);
  await appServerClient.request("terminal/session/resize", {
    ...terminalControlTarget(panel, tab),
    size,
  });
  if (setTerminalTabSize(tab, size)) {
    sendTerminalPanelState(panel);
  }
  return { ok: true };
});

ipcMain.handle("codex:terminal:updatePreferredSize", async (_event, payload) => {
  const threadId = typeof payload?.threadId === "string" ? payload.threadId : "";
  if (!threadId) {
    throw new Error("Thread id is required to update terminal size");
  }
  await appServerClient.request("terminal/preferredSize/update", {
    threadId,
    size: normalizeTerminalSize(payload.size),
  });
  return { ok: true };
});

ipcMain.handle("codex:terminal:terminate", async (event, tabId) => {
  const panel = terminalPanelForEvent(event);
  const tab = requireTerminalTab(panel, tabId);
  if (!terminalTabSupports(tab, "terminate")) {
    throw new Error("Terminal termination is not supported by this execution environment");
  }
  await appServerClient.request(
    "terminal/session/terminate",
    terminalControlTarget(panel, tab),
  );
  return { ok: true };
});

ipcMain.handle("codex:readLocalFile", async (_event, target) => {
  return readLocalFileTarget(target);
});

ipcMain.handle("codex:writeLocalFile", async (_event, target, content) => {
  return writeLocalFileTarget(target, content, defaultWorkspace);
});

ipcMain.handle("codex:listLocalDirectory", async (_event, target) => {
  return listLocalDirectoryTarget(target);
});

ipcMain.handle("codex:readLocalImage", async (_event, target) => {
  return readLocalImageTarget(target);
});

ipcMain.handle("codex:readGitSnapshot", async (_event, cwd, options) => {
  return readGitSnapshot(cwd, options);
});

ipcMain.handle("codex:readGitCommitFiles", async (_event, cwd, hash) => {
  return readGitCommitFiles(cwd, hash);
});

ipcMain.handle("codex:readGitFileDiff", async (_event, cwd, options) => {
  return readGitFileDiff(cwd, options);
});

ipcMain.handle("codex:readGitStatusSnapshot", async (_event, cwd) => {
  return readGitStatusSnapshot(cwd);
});

ipcMain.handle("codex:lspDefinition", async (_event, payload) => {
  return lspManager.definition({
    filePath: payload.path,
    line: payload.line,
    column: payload.column,
  });
});

ipcMain.handle("codex:lspStatus", async (_event, filePath) => {
  return lspManager.status(filePath);
});

ipcMain.handle("codex:sendMessage", async (_event, payload) => {
  const input = buildTurnInput(payload);

  if (payload.expectedTurnId) {
    return appServerClient.request("turn/steer", {
      threadId: payload.threadId,
      expectedTurnId: payload.expectedTurnId,
      input,
    });
  }

  return startThreadTurn(payload, input);
});

ipcMain.handle("codex:interruptTurn", async (_event, payload) => {
  return appServerClient.request("turn/interrupt", {
    threadId: payload.threadId,
    turnId: payload.turnId,
  });
});

ipcMain.handle("codex:respondServerRequest", async (_event, payload) => {
  await appServerClient.respondToServerRequest(payload.requestId, payload.result);
  return { ok: true };
});

ipcMain.handle("codex:rejectServerRequest", async (_event, payload) => {
  await appServerClient.rejectServerRequest(
    payload.requestId,
    payload.message,
    payload.code,
  );
  return { ok: true };
});

ipcMain.handle("codex:requestMicrophoneAccess", async () => {
  if (process.platform !== "darwin") {
    return {
      granted: true,
      status: "granted",
      platform: process.platform,
    };
  }

  const before = systemPreferences.getMediaAccessStatus("microphone");
  if (before === "granted") {
    return {
      granted: true,
      status: before,
      platform: process.platform,
    };
  }
  if (before === "denied" || before === "restricted") {
    return {
      granted: false,
      status: before,
      platform: process.platform,
    };
  }

  const granted = await systemPreferences.askForMediaAccess("microphone");
  const after = systemPreferences.getMediaAccessStatus("microphone");
  return {
    granted: granted || after === "not-determined",
    status: after,
    platform: process.platform,
  };
});

ipcMain.handle("codex:startRealtime", async (_event, payload) => {
  return appServerClient.request("thread/realtime/start", {
    threadId: payload.threadId,
    outputModality: payload.outputModality ?? "text",
    prompt: payload.prompt ?? undefined,
    realtimeSessionId: payload.realtimeSessionId ?? undefined,
    transport: payload.transport,
    voice: payload.voice ?? undefined,
  });
});

ipcMain.handle("codex:stopRealtime", async (_event, payload) => {
  return appServerClient.request("thread/realtime/stop", {
    threadId: payload.threadId,
  });
});

app.whenReady().then(() => {
  registerLocalFilePreviewProtocol();
  startRemoteDebuggingCompatibilityProxy();
  configurePermissionHandlers(session.defaultSession, ({ webContents, permission }) =>
    allowDefaultSessionPermission({
      webContents,
      permission,
      isBrowserPanelWebContents,
    }),
  );
  configurePermissionHandlers(
    session.fromPartition(browserSessionPartition),
    allowBrowserPanelPermission,
  );
  void ensureDefaultWorkspace()
    .then(async () => {
      await appServerClient.ready();
      startupRuntimeRecovery = await recoverLauncherStateAtStartup({
        recordLauncherRecovery: () =>
          recordLauncherRecoveryIfPresent({
            appServerClient,
            evidencePath: process.env.RUNTIME_CAPSULE_FAILURE_EVIDENCE_PATH,
            fs,
            listThreads: () => listThreads(defaultWorkspace),
            subscribeThread,
          }),
        recoverPayloadFailure: () =>
          recoverPayloadRuntimeFailureIfPresent({
            evidencePath: process.env.RUNTIME_CAPSULE_FAILURE_EVIDENCE_PATH,
            formatPayloadRuntimeRecoveryPrompt,
            fs,
            sendSelfCommand: async (text) =>
              sendSelfCommandToThread({
                appServerClient,
                buildTurnInput,
                loadThreadForTurn: async (threadId) =>
                  (await subscribeThread(threadId)).thread ?? null,
                normalizeThread,
                persistSystemThreadId: persistCurrentSystemSelfThreadId,
                project: await ensureSelfProjectForCurrentApp(),
                rememberThreadRuntime,
                startThreadTurn,
                text,
                threads: await listAllThreads(appServerClient, normalizeThread),
              }),
          }),
      });
      const window = await createWindow();
      // Keep the first window visible before a system permission prompt can
      // wait for user input.
      void primeMicrophoneAccessPrompt();
      return window;
    })
    .catch(handleStartupError);
  app.on("activate", () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      void ensureDefaultWorkspace()
        .then(() => createWindow())
        .catch(handleStartupError);
    }
  });
});

app.on("window-all-closed", () => {
  if (process.platform !== "darwin") {
    app.quit();
  }
});

app.on("before-quit", (event) => {
  if (quittingAfterAppServerStop || !appServerClient.status.connected) {
    return;
  }
  quittingAfterAppServerStop = true;
  event.preventDefault();
  void appServerClient
    .stop("application quit")
    .catch((error) => {
      console.error(
        "[prototype] app-server stop during app quit failed",
        JSON.stringify({
          reason: error instanceof Error ? error.message : String(error),
        }),
      );
    })
    .finally(() => {
      app.exit(0);
    });
});

function browserPanelForEvent(event) {
  const window = BrowserWindow.fromWebContents(event.sender);
  if (!window) {
    throw new Error("Browser panel requires an active application window");
  }
  return browserPanelForWindow(window);
}

function terminalPanelForEvent(event) {
  const window = BrowserWindow.fromWebContents(event.sender);
  if (!window) {
    throw new Error("Terminal panel requires an active application window");
  }
  const existing = terminalPanelsByWindowId.get(window.id);
  if (existing) {
    return existing;
  }
  const panel = {
    window,
    state: createTerminalPanelState(commandOutputCache),
    error: null,
    threadId: null,
    refreshPromise: null,
    refreshThreadId: null,
    refreshToken: null,
    pendingNotifications: [],
    userTerminalResumeTokens: new Map(),
  };
  terminalPanelsByWindowId.set(window.id, panel);
  return panel;
}

function terminalPanelState(panel) {
  return {
    ...terminalPanelSnapshot(panel.state),
    error: panel.error,
  };
}

function sendTerminalPanelState(panel, event = { type: "snapshot" }) {
  if (!panel.window.isDestroyed()) {
    panel.window.webContents.send("codex:terminal:state", {
      ...event,
      state: terminalPanelState(panel),
    });
  }
}

function routeTerminalNotification(notification) {
  cacheTerminalNotification(notification);
  for (const panel of terminalPanelsByWindowId.values()) {
    if (panel.refreshPromise) {
      panel.pendingNotifications.push(notification);
      if (
        panel.pendingNotifications.length >
        MAX_PENDING_TERMINAL_NOTIFICATIONS
      ) {
        panel.pendingNotifications.splice(
          0,
          panel.pendingNotifications.length -
            MAX_PENDING_TERMINAL_NOTIFICATIONS,
        );
      }
    } else {
      applyTerminalNotification(panel, notification, true);
    }
  }
}

function cacheTerminalNotification(notification) {
  const params = notification.params ?? {};
  if (notification.method === "item/commandExecution/outputDelta") {
    appendCommandOutputCache(
      { commandOutputCache },
      params.threadId,
      params.itemId,
      params.deltaBase64,
      params.sequence,
    );
    return;
  }
  if (
    notification.method === "item/completed" &&
    params.item?.type === "commandExecution"
  ) {
    deleteCommandOutputCache(
      { commandOutputCache },
      params.threadId,
      params.item.id,
    );
  }
}

function applyTerminalNotification(panel, notification, allowRefresh) {
  const params = notification.params ?? {};
  if (notification.method === "command/exec/started") {
    const tab = panel.state.tabs.find(
      (candidate) =>
        candidate.origin === "user" &&
        candidate.processId === params.processId &&
        candidate.status === "starting",
    );
    if (tab) {
      tab.generation = params.generation;
      tab.status = "running";
      panel.userTerminalResumeTokens.set(tab.processId, params.resumeToken);
      sendTerminalPanelState(panel);
    }
    return;
  }
  if (
    notification.method === "item/started" &&
    params.item?.type === "commandExecution" &&
    panel.threadId === params.threadId &&
    allowRefresh
  ) {
    // A PTY is interactive as soon as the process starts, even before it
    // writes output. Listing is the runtime fact that makes the tab available.
    void refreshTerminalPanelSessions(panel);
    return;
  }
  if (notification.method === "command/exec/outputDelta") {
    const tab = panel.state.tabs.find(
      (candidate) =>
        candidate.origin === "user" &&
        candidate.processId === params.processId &&
        candidate.generation === params.generation,
    );
    if (
      tab &&
      appendTerminalOutput(tab, params.deltaBase64, params.sequence)
    ) {
      tab.backgroundActivity = panel.state.activeTabId !== tab.id;
      sendTerminalPanelDelta(panel, {
        type: "delta",
        tabId: tab.id,
        deltaBase64: params.deltaBase64,
        tab: terminalTabMetadata(tab),
      });
      if (tab.hasSequenceGap && allowRefresh) {
        void refreshTerminalPanelSessions(panel);
      }
    } else if (
      !tab &&
      allowRefresh &&
      !isTerminalSessionDetached(panel.state, {
        origin: "user",
        threadId: null,
        processId: params.processId,
        generation: params.generation,
      })
    ) {
      void refreshTerminalPanelSessions(panel);
    }
    return;
  }
  if (notification.method === "command/exec/exited") {
    const tab = panel.state.tabs.find(
      (candidate) =>
        candidate.origin === "user" &&
        candidate.processId === params.processId &&
        candidate.generation === params.generation,
    );
    if (tab) {
      markTerminalExited(tab, params.exitCode);
      sendTerminalPanelState(panel);
    }
    return;
  }
  if (notification.method === "item/commandExecution/outputDelta") {
    const tab = panel.state.tabs.find(
      (candidate) =>
        candidate.origin === "model" &&
        candidate.threadId === params.threadId &&
        candidate.commandItemId === params.itemId &&
        (candidate.processId === params.processId ||
          candidate.processId === params.itemId),
    );
    if (
      tab &&
      candidateProcessIdIsPlaceholder(tab.processId, params.itemId) &&
      typeof params.processId === "string" &&
      params.processId.length > 0
    ) {
      tab.processId = params.processId;
    }
    if (
      tab &&
      params.deltaBase64 &&
      appendTerminalOutput(tab, params.deltaBase64, params.sequence)
    ) {
      const needsLiveRefresh = terminalTabNeedsLiveSessionRefresh(tab);
      tab.backgroundActivity = panel.state.activeTabId !== tab.id;
      sendTerminalPanelDelta(panel, {
        type: "delta",
        tabId: tab.id,
        deltaBase64: params.deltaBase64,
        tab: terminalTabMetadata(tab),
      });
      if (allowRefresh && (tab.hasSequenceGap || needsLiveRefresh)) {
        void refreshTerminalPanelSessions(panel, params.threadId);
      }
    } else if (
      !tab &&
      allowRefresh &&
      panel.threadId === params.threadId &&
      !isTerminalSessionDetached(panel.state, {
        origin: "model",
        threadId: params.threadId,
        processId: params.processId,
        generation: params.itemId,
      })
    ) {
      void refreshTerminalPanelSessions(panel);
    }
    return;
  }
  if (
    notification.method === "item/completed" &&
    params.item?.type === "commandExecution"
  ) {
    deleteCommandOutputCache(
      { commandOutputCache },
      params.threadId,
      params.item.id,
    );
    const tab = panel.state.tabs.find(
      (candidate) =>
        candidate.origin === "model" &&
        candidate.threadId === params.threadId &&
        candidate.commandItemId === params.item.id,
    );
    if (tab) {
      markTerminalExited(tab, params.item.exitCode);
      sendTerminalPanelState(panel);
    }
  }
}

function sendTerminalPanelDelta(panel, event) {
  if (!panel.window.isDestroyed()) {
    panel.window.webContents.send("codex:terminal:state", event);
  }
}

async function refreshTerminalPanelSessions(panel, threadId = panel.threadId) {
  if (panel.refreshPromise && panel.refreshThreadId === threadId) {
    return panel.refreshPromise;
  }
  const refreshToken = {};
  panel.refreshThreadId = threadId;
  panel.refreshToken = refreshToken;
  const refreshPromise = Promise.resolve().then(async () => {
    try {
      const response = await appServerClient.request("terminal/session/list", {
        threadId,
        userResumeTokens: [...panel.userTerminalResumeTokens.values()],
      });
      if (panel.threadId !== threadId) {
        return;
      }
      mergeTerminalSessions(panel.state, response.data ?? [], threadId);
      const pendingNotifications = panel.pendingNotifications.splice(0);
      for (const notification of pendingNotifications) {
        applyTerminalNotification(panel, notification, false);
      }
      panel.error = null;
      return true;
    } catch (error) {
      if (panel.threadId === threadId) {
        panel.error = error instanceof Error ? error.message : String(error);
      }
      return false;
    } finally {
      if (panel.refreshToken === refreshToken) {
        panel.refreshPromise = null;
        panel.refreshThreadId = null;
        panel.refreshToken = null;
      }
      if (panel.threadId === threadId) {
        sendTerminalPanelState(panel);
      }
    }
  });
  panel.refreshPromise = refreshPromise;
  return refreshPromise;
}

function requireTerminalTab(panel, tabId) {
  const tab = panel.state.tabs.find((candidate) => candidate.id === tabId);
  if (!tab) {
    throw new Error("Terminal tab not found");
  }
  if (tab.status !== "running" && tab.status !== "starting") {
    throw new Error("Terminal session is no longer running");
  }
  return tab;
}

function terminalControlTarget(panel, tab) {
  return {
    sessionId: tab.sessionId,
    generation: tab.generation,
    origin: tab.origin,
    threadId: tab.threadId,
    commandItemId: tab.commandItemId,
    processId: tab.processId,
    resumeToken:
      tab.origin === "user"
        ? panel.userTerminalResumeTokens.get(tab.processId) ?? null
        : null,
  };
}

function normalizeTerminalSize(size) {
  return {
    rows: Math.max(1, Math.min(65535, Math.round(Number(size?.rows) || 24))),
    cols: Math.max(1, Math.min(65535, Math.round(Number(size?.cols) || 80))),
  };
}

function candidateProcessIdIsPlaceholder(processId, commandItemId) {
  return (
    typeof processId === "string" &&
    processId.length > 0 &&
    processId === commandItemId
  );
}

function resolveInteractiveShell() {
  if (process.platform === "win32") {
    return process.env.COMSPEC || "cmd.exe";
  }
  return process.env.SHELL || "/bin/zsh";
}

function browserPanelForWindow(window) {
  const existing = browserPanelsByWindowId.get(window.id);
  if (existing) {
    return existing;
  }

  if (typeof WebContentsView !== "function") {
    throw new Error("This Electron version does not support WebContentsView");
  }

  const initialBoundsUpdate = normalizeBrowserBoundsUpdate(null);
  const panel = {
    window,
    visible: false,
    bounds: initialBoundsUpdate.bounds,
    boundsSequence: initialBoundsUpdate.sequence,
    tabs: [],
    activeTabId: null,
    attachedTabId: null,
    destroying: false,
  };

  browserPanelsByWindowId.set(window.id, panel);
  createBrowserPanelTab(panel, { activate: true });
  return panel;
}

function startRemoteDebuggingCompatibilityProxy() {
  if (!remoteDebuggingConfig.enabled || !remoteDebuggingConfig.proxy?.enabled) {
    return;
  }
  remoteDebuggingProxy = startRemoteDebuggingProxy({
    ...remoteDebuggingConfig.proxy,
    createTarget: createBrowserPanelDebugTarget,
    logger: console,
  });
}

async function createBrowserPanelDebugTarget(target) {
  const targetRequest = normalizeBrowserDebugTarget(target);
  if (!targetRequest.ok) {
    throw new Error(targetRequest.reason);
  }

  const window = firstAvailableWindow();
  if (!window) {
    throw new Error("No Root Worker window is available for Browser panel tabs");
  }

  const panel = browserPanelForWindow(window);
  const tab = createBrowserPanelTab(panel, { activate: true });
  try {
    if (targetRequest.url) {
      void loadBrowserPanelTabUrl(panel, tab, targetRequest.url).catch((error) => {
        tab.state.loading = false;
        tab.state.error = error instanceof Error ? error.message : String(error);
        sendBrowserPanelState(panel);
      });
    } else {
      await loadBrowserPanelTabAboutBlankBootstrap(panel, tab);
    }
    sendBrowserPanelState(panel);
    const targetId = await waitForBrowserPanelDevToolsTarget(tab.view.webContents);
    return { targetId, tabId: tab.id };
  } catch (error) {
    closeBrowserPanelTab(panel, tab.id);
    sendBrowserPanelState(panel);
    throw error;
  }
}

async function loadBrowserPanelTabAboutBlankBootstrap(panel, tab) {
  tab.allowNextAboutBlankNavigation = true;
  try {
    await tab.view.webContents.loadURL("about:blank");
  } finally {
    tab.allowNextAboutBlankNavigation = false;
    updateBrowserPanelLocationState(tab);
    sendBrowserPanelState(panel);
  }
}

async function waitForBrowserPanelDevToolsTarget(webContents) {
  const deadline = Date.now() + 5_000;
  let lastError = null;
  while (Date.now() < deadline) {
    try {
      const targets = await fetchRemoteDebuggingJson("/json/list");
      const target = targets.find((candidate) => {
        if (!candidate || candidate.type !== "page" || !candidate.id) {
          return false;
        }
        return electronWebContents.fromDevToolsTargetId(candidate.id) === webContents;
      });
      if (target) {
        return target.id;
      }
    } catch (error) {
      lastError = error;
    }
    await delay(100);
  }

  const suffix = lastError
    ? `: ${lastError instanceof Error ? lastError.message : String(lastError)}`
    : "";
  throw new Error(`Browser panel DevTools target was not published${suffix}`);
}

function fetchRemoteDebuggingJson(pathname) {
  return new Promise((resolve, reject) => {
    const request = http.request(
      {
        host: remoteDebuggingConfig.address,
        port: Number(remoteDebuggingConfig.backendPort),
        path: pathname,
        method: "GET",
      },
      (response) => {
        const chunks = [];
        response.on("data", (chunk) => chunks.push(chunk));
        response.on("end", () => {
          if ((response.statusCode ?? 500) >= 400) {
            reject(
              new Error(
                `remote debugging backend returned HTTP ${response.statusCode}`,
              ),
            );
            return;
          }
          try {
            resolve(JSON.parse(Buffer.concat(chunks).toString("utf8")));
          } catch (error) {
            reject(error);
          }
        });
      },
    );
    request.on("error", reject);
    request.end();
  });
}

function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function firstAvailableWindow() {
  for (const window of windows) {
    if (!window.isDestroyed()) {
      return window;
    }
  }
  return null;
}

function attachBrowserPanel(panel) {
  if (panel.visible) {
    return;
  }
  panel.visible = true;
  attachActiveBrowserPanelView(panel);
}

function detachBrowserPanel(panel) {
  if (!panel.visible) {
    return;
  }
  detachAttachedBrowserPanelView(panel);
  panel.visible = false;
}

function destroyBrowserPanel(window) {
  const panel = browserPanelsByWindowId.get(window.id);
  if (!panel) {
    return;
  }
  panel.destroying = true;
  browserPanelsByWindowId.delete(window.id);
  detachBrowserPanel(panel);
  for (const tab of panel.tabs) {
    closeBrowserPanelTabContents(tab);
  }
  panel.tabs = [];
  panel.activeTabId = null;
}

function setBrowserPanelBounds(panel, bounds) {
  const update = normalizeBrowserBoundsUpdate(bounds, panel.boundsSequence);
  panel.boundsSequence = update.sequence;
  if (!update.apply) {
    return;
  }
  panel.bounds = update.bounds;
  const tab = activeBrowserPanelTab(panel);
  if (
    panel.visible &&
    tab &&
    !panel.window.isDestroyed() &&
    !tab.view.webContents.isDestroyed()
  ) {
    tab.view.setBounds(panel.bounds);
  }
}

function sendBrowserPanelState(panel) {
  if (!panel.window.isDestroyed()) {
    panel.window.webContents.send("codex:browser:state", browserPanelState(panel));
  }
}

function browserPanelState(panel) {
  const activeTab = activeBrowserPanelTab(panel);
  if (activeTab) {
    updateBrowserPanelLocationState(activeTab);
  }
  const activeState = activeTab?.state ?? emptyBrowserPanelTabState();
  return {
    ...activeState,
    activeTabId: panel.activeTabId,
    tabs: panel.tabs.map((tab) => {
      updateBrowserPanelLocationState(tab);
      return {
        id: tab.id,
        ...tab.state,
      };
    }),
  };
}

function emptyBrowserPanelTabState() {
  return {
    url: null,
    title: null,
    loading: false,
    canGoBack: false,
    canGoForward: false,
    error: null,
  };
}

function createBrowserPanelTab(panel, { url = null, activate = true } = {}) {
  const view = new WebContentsView({
    webPreferences: browserPanelWebPreferences(),
  });
  const tab = {
    id: `browser-tab-${++browserPanelTabCounter}`,
    view,
    state: emptyBrowserPanelTabState(),
    allowNextAboutBlankNavigation: false,
  };
  panel.tabs.push(tab);
  bindBrowserPanelTab(panel, tab);
  if (activate || !panel.activeTabId) {
    selectBrowserPanelTab(panel, tab.id);
  }
  if (url) {
    void loadBrowserPanelTabUrl(panel, tab, url).catch((error) => {
      tab.state.loading = false;
      tab.state.error = error instanceof Error ? error.message : String(error);
      sendBrowserPanelState(panel);
    });
  }
  return tab;
}

async function loadBrowserPanelTabUrl(panel, tab, target) {
  const normalized = normalizeBrowserTarget(target);
  if (!normalized.ok) {
    tab.state.error = normalized.reason;
    tab.state.loading = false;
    sendBrowserPanelState(panel);
    throw new Error(normalized.reason);
  }
  tab.state.error = null;
  await tab.view.webContents.loadURL(normalized.url);
}

function bindBrowserPanelTab(panel, tab) {
  tab.view.webContents.setWindowOpenHandler(({ url }) => {
    const normalized = normalizeBrowserTarget(url);
    if (normalized.ok) {
      createBrowserPanelTab(panel, { url: normalized.url, activate: true });
      sendBrowserPanelState(panel);
    }
    return { action: "deny" };
  });

  tab.view.webContents.on("will-navigate", (event, url) =>
    guardBrowserPanelNavigation(panel, tab, event, url),
  );
  tab.view.webContents.on("will-frame-navigate", (event, url) =>
    guardBrowserPanelNavigation(panel, tab, event, url),
  );
  tab.view.webContents.on("will-redirect", (event, url) =>
    guardBrowserPanelNavigation(panel, tab, event, url),
  );
  tab.view.webContents.on("did-start-loading", () => {
    tab.state.loading = true;
    tab.state.error = null;
    sendBrowserPanelState(panel);
  });
  tab.view.webContents.on("did-stop-loading", () => {
    updateBrowserPanelLocationState(tab);
    tab.state.loading = false;
    sendBrowserPanelState(panel);
  });
  tab.view.webContents.on("did-navigate", (_event, url) => {
    tab.state.url = url || null;
    updateBrowserPanelLocationState(tab);
    sendBrowserPanelState(panel);
  });
  tab.view.webContents.on("did-navigate-in-page", (_event, url) => {
    tab.state.url = url || null;
    updateBrowserPanelLocationState(tab);
    sendBrowserPanelState(panel);
  });
  tab.view.webContents.on("page-title-updated", (_event, title) => {
    tab.state.title = title || null;
    sendBrowserPanelState(panel);
  });
  tab.view.webContents.on(
    "did-fail-load",
    (_event, errorCode, errorDescription, validatedUrl, isMainFrame) => {
      if (!isMainFrame || errorCode === -3) {
        return;
      }
      tab.state.url = validatedUrl || tab.state.url;
      tab.state.loading = false;
      tab.state.error = errorDescription || "Page failed to load";
      updateBrowserPanelLocationState(tab);
      sendBrowserPanelState(panel);
    },
  );
  tab.view.webContents.on("destroyed", () => {
    removeDestroyedBrowserPanelTab(panel, tab);
  });
}

function activeBrowserPanelTab(panel) {
  return panel.tabs.find((tab) => tab.id === panel.activeTabId) ?? panel.tabs[0] ?? null;
}

function selectBrowserPanelTab(panel, tabId) {
  const nextTab = panel.tabs.find((tab) => tab.id === tabId);
  if (!nextTab) {
    return false;
  }
  if (panel.activeTabId === nextTab.id) {
    if (panel.visible) {
      attachActiveBrowserPanelView(panel);
    }
    return true;
  }
  detachAttachedBrowserPanelView(panel);
  panel.activeTabId = nextTab.id;
  if (panel.visible) {
    attachActiveBrowserPanelView(panel);
  }
  return true;
}

function closeBrowserPanelTab(panel, tabId) {
  const index = panel.tabs.findIndex((tab) => tab.id === tabId);
  if (index === -1) {
    return false;
  }
  const tab = panel.tabs[index];
  const wasActive = panel.activeTabId === tab.id;
  const nextActiveTabId = nextBrowserTabIdAfterClose(
    panel.tabs,
    panel.activeTabId,
    tab.id,
  );
  if (wasActive) {
    detachAttachedBrowserPanelView(panel);
  }
  panel.tabs.splice(index, 1);
  closeBrowserPanelTabContents(tab);
  if (panel.tabs.length === 0 && !panel.destroying) {
    createBrowserPanelTab(panel, { activate: true });
    return true;
  }
  if (wasActive) {
    panel.activeTabId = nextActiveTabId;
    if (panel.visible) {
      attachActiveBrowserPanelView(panel);
    }
  }
  return true;
}

function removeDestroyedBrowserPanelTab(panel, tab) {
  const index = panel.tabs.findIndex((candidate) => candidate.id === tab.id);
  if (index === -1) {
    return;
  }
  const wasActive = panel.activeTabId === tab.id;
  panel.tabs.splice(index, 1);
  if (panel.attachedTabId === tab.id) {
    panel.attachedTabId = null;
  }
  if (panel.destroying || panel.window.isDestroyed()) {
    return;
  }
  if (panel.tabs.length === 0) {
    createBrowserPanelTab(panel, { activate: true });
  } else if (wasActive) {
    const nextTab = panel.tabs[Math.min(index, panel.tabs.length - 1)] ?? panel.tabs[0];
    panel.activeTabId = nextTab.id;
    if (panel.visible) {
      attachActiveBrowserPanelView(panel);
    }
  }
  sendBrowserPanelState(panel);
}

function closeBrowserPanelTabContents(tab) {
  if (!tab.view.webContents.isDestroyed()) {
    tab.view.webContents.close({ waitForBeforeUnload: false });
  }
}

function attachActiveBrowserPanelView(panel) {
  const tab = activeBrowserPanelTab(panel);
  if (
    !tab ||
    !panel.visible ||
    panel.window.isDestroyed() ||
    tab.view.webContents.isDestroyed()
  ) {
    return;
  }
  if (panel.attachedTabId === tab.id) {
    tab.view.setBounds(panel.bounds);
    return;
  }
  detachAttachedBrowserPanelView(panel);
  panel.window.contentView.addChildView(tab.view);
  panel.attachedTabId = tab.id;
  tab.view.setBounds(panel.bounds);
}

function detachAttachedBrowserPanelView(panel) {
  const attachedTabId = panel.attachedTabId;
  panel.attachedTabId = null;
  const tab = panel.tabs.find((candidate) => candidate.id === attachedTabId);
  const tabDestroyed = !tab || tab.view.webContents.isDestroyed();
  if (
    shouldDetachAttachedBrowserPanelView({
      attachedTabId,
      tabDestroyed,
      windowDestroyed: panel.window.isDestroyed(),
    })
  ) {
    panel.window.contentView.removeChildView(tab.view);
  }
}

function updateBrowserPanelLocationState(tab) {
  if (!tab.view.webContents.isDestroyed()) {
    const navigation = browserNavigation(tab.view.webContents);
    tab.state.url = tab.view.webContents.getURL() || tab.state.url;
    tab.state.title = tab.view.webContents.getTitle() || tab.state.title;
    tab.state.loading = tab.view.webContents.isLoading();
    tab.state.canGoBack = navigation.canGoBack();
    tab.state.canGoForward = navigation.canGoForward();
  }
}

function browserNavigation(webContents) {
  return {
    canGoBack: () =>
      webContents.navigationHistory?.canGoBack?.() ?? webContents.canGoBack(),
    canGoForward: () =>
      webContents.navigationHistory?.canGoForward?.() ??
      webContents.canGoForward(),
    goBack: () =>
      webContents.navigationHistory?.goBack?.() ?? webContents.goBack(),
    goForward: () =>
      webContents.navigationHistory?.goForward?.() ?? webContents.goForward(),
  };
}

function isBrowserPanelWebContents(webContents) {
  for (const panel of browserPanelsByWindowId.values()) {
    for (const tab of panel.tabs) {
      if (tab.view.webContents === webContents) {
        return true;
      }
    }
  }
  return false;
}

function guardBrowserPanelNavigation(panel, tab, event, target) {
  const eventTarget = browserNavigationEventTarget(event, target);
  if (tab.allowNextAboutBlankNavigation && eventTarget === "about:blank") {
    return;
  }
  const decision = browserNavigationEventDecision(event, target);
  if (decision.allow) {
    return;
  }

  event.preventDefault();
  tab.state.error = decision.reason;
  tab.state.loading = false;
  sendBrowserPanelState(panel);
}

async function ensureBuiltRenderer() {
  const rendererPath = builtRendererPath();
  try {
    await fs.access(rendererPath);
  } catch (error) {
    throw new Error(
      `Built renderer not found at ${rendererPath}. Run 'pnpm --filter @my-codex/root-worker-prototype build' before 'pnpm --filter @my-codex/root-worker-prototype start'.`,
      { cause: error },
    );
  }
}

function builtRendererPath() {
  return path.join(__dirname, "../dist/index.html");
}

async function handleStartupError(error) {
  console.error("[prototype] failed to start renderer", error);
  await writePayloadFailureEvidence({
    fs,
    reason: error instanceof Error ? error.message : String(error),
  }).catch((evidenceError) => {
    console.error(
      "[prototype] failed to write payload failure evidence",
      evidenceError,
    );
  });
  app.exit(1);
}

function requestFatalPayloadExit(error) {
  if (fatalPayloadExitRequested) {
    return;
  }
  fatalPayloadExitRequested = true;
  void handleStartupError(error);
}

function requestClientRelaunch(reason) {
  const result = appRelaunch.requestRelaunch(reason);
  if (!result.ok) {
    console.error(
      "[prototype] client relaunch unavailable",
      JSON.stringify({ reason: result.reason }),
    );
  }
  return result;
}

async function reloadRendererWindows() {
  let windowsReloaded = 0;
  const reloads = [];
  for (const window of windows) {
    if (window.isDestroyed()) {
      continue;
    }
    windowsReloaded += 1;
    reloads.push(reloadWindowRenderer(window));
  }
  if (windowsReloaded === 0) {
    throw new Error("No renderer windows are available to reload");
  }
  await Promise.all(reloads);
  return { windowsReloaded };
}

function reloadWindowRenderer(window) {
  return new Promise((resolve, reject) => {
    const webContents = window.webContents;
    if (webContents.isDestroyed()) {
      reject(new Error("Renderer webContents is destroyed"));
      return;
    }
    const cleanup = () => {
      webContents.removeListener("did-finish-load", handleFinish);
      webContents.removeListener("did-fail-load", handleFail);
    };
    const handleFinish = () => {
      cleanup();
      resolve();
    };
    const handleFail = (
      _event,
      _errorCode,
      errorDescription,
      _url,
      isMainFrame,
    ) => {
      if (!isMainFrame) {
        return;
      }
      cleanup();
      reject(new Error(errorDescription || "Renderer reload failed"));
    };
    webContents.once("did-finish-load", handleFinish);
    webContents.on("did-fail-load", handleFail);
    try {
      if (!isDev) {
        void window
          .loadFile(builtRendererPath())
          .catch((error) => {
            cleanup();
            reject(error);
          });
      } else if (typeof webContents.reloadIgnoringCache === "function") {
        webContents.reloadIgnoringCache();
      } else {
        webContents.reload();
      }
    } catch (error) {
      cleanup();
      reject(error);
    }
  });
}

function getAutoResumeCoordinator() {
  if (!autoResumeCoordinator) {
    autoResumeCoordinator = createThreadAutoResumeCoordinator({
      readThread,
      subscribeThread,
      sendResumeInput: sendAutoResumeInput,
      stateStore: createJsonAutoResumeStateStore(
        path.join(app.getPath("userData"), "runtime-autoresume.json"),
        fs,
      ),
      logger: console,
    });
  }
  return autoResumeCoordinator;
}

function getRuntimeRestartIntentStore() {
  if (!runtimeRestartIntentStore) {
    runtimeRestartIntentStore = createRuntimeRestartIntentStore(
      path.join(app.getPath("userData"), "runtime-restart-intents.json"),
      { fs },
    );
  }
  return runtimeRestartIntentStore;
}

function getRuntimeRestartController() {
  if (!runtimeRestartController) {
    runtimeRestartController = createRuntimeRestartController({
      store: getRuntimeRestartIntentStore(),
      execute: (notification, handoff = null) =>
        observeClientRelaunchResult(
          handleClientRelaunchNotification({
            ...notification,
            params: {
              ...(notification?.params ?? {}),
              runtimeRestartHandoff: handoff,
            },
          }),
          {
            broadcastStatus: (status) =>
              broadcast("codex:status", {
                ...appServerClient.status,
                ...status,
              }),
            logger: console,
            reason:
              notification.params?.reason ?? notification.method ?? null,
            requestId: notification.params?.requestId ?? null,
          },
        ),
      recover: recoverRuntimeRestartRecord,
      broadcastStatus: (status) =>
        broadcast("codex:status", {
          ...appServerClient.status,
          ...status,
        }),
      logger: console,
    });
  }
  return runtimeRestartController;
}

async function recoverRuntimeRestartRecord(record) {
  if (!shouldNotifyRuntimeRestartRecoveryOnSelf(record)) {
    return;
  }
  return notifyRecoverableRestartErrorOnSelf({
    sourceThreadId: record.requestedByThreadId,
    noticeId: runtimeRestartRecoveryNoticeId(record),
    prompt: expectedRuntimeRestartRecoveryPrompt(record),
    readThread,
    subscribeThread,
    submitRecoveryMessage: (thread, message) =>
      submitRuntimeRestartRecoveryMessage(thread, message),
  });
}

function runtimeRestartRecoveryNoticeId(record) {
  const requestId =
    typeof record?.requestId === "string" && record.requestId.trim()
      ? record.requestId.trim()
      : "unknown";
  return `runtime-restart-recovery:${requestId}`;
}

async function submitRuntimeRestartRecoveryMessage(thread, message) {
  const threadId = typeof thread?.id === "string" ? thread.id.trim() : "";
  if (!threadId) {
    throw new Error("recovery notice requires a thread id");
  }
  const text =
    typeof message?.text === "string" && message.text.trim()
      ? message.text.trim()
      : null;
  if (!text) {
    throw new Error("recovery notice requires text");
  }
  return startThreadTurn({
    threadId,
    model: thread?.model ?? null,
    modelProvider: thread?.modelProvider ?? null,
    effort: thread?.reasoningEffort ?? null,
    text,
    skills: [],
    images: [],
  });
}

async function sendAutoResumeInput(thread, text) {
  return startThreadTurn({
    threadId: thread.id,
    model: thread?.model ?? null,
    modelProvider: thread?.modelProvider ?? null,
    effort: thread?.reasoningEffort ?? null,
    text,
    skills: [],
    images: [],
  });
}

async function listThreads(cwd) {
  const threads = await listAllThreads(appServerClient, normalizeThread);
  const project = await ensureSelfProjectForCurrentApp();
  if (!project) {
    return {
      materializedSelfThreadId: null,
      selfProjectThreadId: null,
      threads,
    };
  }
  const result = await ensureSelfProjectThread(
    appServerClient,
    normalizeThread,
    persistCurrentSystemSelfThreadId,
    project,
    threads,
  );
  if (result.created) {
    rememberThreadRuntime(result.thread.id, result.runtime);
  }
  return {
    materializedSelfThreadId: result.created ? result.thread.id : null,
    selfProjectThreadId: result.thread.id,
    threads: result.threads,
  };
}

async function listSkills(cwd) {
  const response = await appServerClient.request("skills/list", {
    cwds: [cwd],
  });
  const entry = response.data?.[0];
  return {
    skills: (entry?.skills ?? []).map(normalizeAvailableSkill),
    errors: (entry?.errors ?? []).map(
      (error) => error.message ?? String(error),
    ),
  };
}

async function readThread(threadId, includeTurns, runtime = null, options = {}) {
  const response = await appServerClient.request("thread/read", {
    threadId,
    includeTurns,
  });
  return { thread: normalizeThread(response.thread, runtime, options) };
}

async function subscribeThread(threadId) {
  const response = await appServerClient.request("thread/read", {
    threadId,
    includeTurns: false,
  });
  const existingRuntime = threadRuntimeById.get(threadId) ?? null;
  const runtime = resolveRuntimeForResume(existingRuntime, response.thread ?? response);
  rememberThreadRuntime(threadId, runtime);
  return {
    thread: response.thread
      ? normalizeThread({ ...response.thread, turns: [] }, runtime)
      : null,
  };
}

async function unsubscribeThread(threadId) {
  const response = await appServerClient.request("thread/unsubscribe", {
    threadId,
  });
  const status = response.status === "unsubscribed" ? "unsubscribed" : "notSubscribed";
  return { status };
}

async function ensureSelfProjectForCurrentApp() {
  if (!isPackagedApp()) {
    return null;
  }
  const workspace = resolveDefaultWorkspace(process.env, {
    isPackagedApp: true,
    sourceOnly: true,
  });
  if (!workspace) {
    return null;
  }
  return ensureSelfProjectSync(process.env, workspace);
}

function persistCurrentSystemSelfThreadId(threadId) {
  return recordSystemSelfThreadIdSync(process.env, threadId);
}

async function startThreadTurn(payload, input = buildTurnInput(payload)) {
  const response = await appServerClient.request(
    "turn/start",
    buildTurnStartParams(payload, input),
  );
  rememberThreadRuntime(
    payload.threadId,
    mergeRuntimeOverride(threadRuntimeById.get(payload.threadId), payload),
  );
  return response;
}

function rememberThreadRuntime(threadId, runtime) {
  threadRuntimeById.set(threadId, runtime);
}

function normalizeNotification(notification) {
  if (notification.method === "thread/started") {
    return {
      ...notification,
      params: {
        thread: normalizeThread(notification.params.thread),
      },
    };
  }

  if (notification.method === "thread/skills/updated") {
    return {
      ...notification,
      params: {
        ...notification.params,
        skills: (notification.params.skills ?? []).map(normalizeThreadSkill),
      },
    };
  }

  if (notification.method === "thread/tokenUsage/updated") {
    return {
      ...notification,
      params: {
        ...notification.params,
        tokenUsage: normalizeThreadTokenUsage(notification.params.tokenUsage),
      },
    };
  }

  if (notification.method === "thread/contextUsage/updated") {
    return {
      ...notification,
      params: {
        ...notification.params,
        tokenUsage: normalizeThreadTokenUsage(notification.params.tokenUsage),
        contextUsage: normalizeThreadContextUsage(
          notification.params.contextUsage,
        ),
      },
    };
  }

  if (notification.method === "thread/status/changed") {
    return {
      ...notification,
      params: {
        ...notification.params,
        lifecycleStatus: normalizeThreadLifecycleStatus(
          notification.params.lifecycleStatus ?? notification.params.status,
        ),
      },
    };
  }

  if (notification.method === "thread/goal/updated") {
    return {
      ...notification,
      params: {
        ...notification.params,
        goal: normalizeThreadGoal(notification.params.goal),
      },
    };
  }

  if (
    notification.method === "turn/started" ||
    notification.method === "turn/completed"
  ) {
    return {
      ...notification,
      params: {
        ...notification.params,
        turn: normalizeTurn(notification.params.turn),
      },
    };
  }

  if (
    notification.method === "item/started" ||
    notification.method === "item/completed"
  ) {
    return {
      ...notification,
      params: {
        ...notification.params,
        item: normalizeItem(notification.params.item),
      },
    };
  }

  return notification;
}

function normalizeRequest(request) {
  return request;
}

function isApprovalServerRequest(request) {
  return (
    request?.method === "item/commandExecution/requestApproval" ||
    request?.method === "item/fileChange/requestApproval" ||
    request?.method === "item/permissions/requestApproval"
  );
}

function normalizeThread(thread, runtime = null, options = {}) {
  const tokenUsage = Object.prototype.hasOwnProperty.call(thread, "tokenUsage")
    ? thread.tokenUsage
      ? normalizeThreadTokenUsage(thread.tokenUsage)
      : null
    : null;
  const contextUsage = Object.prototype.hasOwnProperty.call(
    thread,
    "contextUsage",
  )
    ? thread.contextUsage
      ? normalizeThreadContextUsage(thread.contextUsage)
      : null
    : null;
  const threadUsage = { tokenUsage, contextUsage };

  return normalizeThreadSnapshot({
    ...thread,
    model: runtime?.model ?? thread.model ?? null,
    modelProvider: runtime?.modelProvider ?? thread.modelProvider ?? null,
    reasoningEffort: runtime?.reasoningEffort ?? thread.reasoningEffort ?? null,
    lifecycleStatus: normalizeThreadLifecycleStatus(
      thread.lifecycleStatus ?? thread.status,
    ),
    skills: (thread.skills ?? []).map(normalizeThreadSkill),
    threadUsage,
    ...(Object.prototype.hasOwnProperty.call(thread, "tokenUsage")
      ? { tokenUsage }
      : {}),
    ...(Object.prototype.hasOwnProperty.call(thread, "contextUsage")
      ? { contextUsage }
      : {}),
    turns: (thread.turns ?? []).map((turn) => normalizeTurn(turn, options)),
  });
}

function normalizeThreadSkill(skill) {
  return {
    ...skill,
    kind: normalizeThreadSkillKind(skill?.kind),
  };
}

function normalizeThreadContextUsage(contextUsage) {
  if (!contextUsage || typeof contextUsage !== "object") {
    return null;
  }

  return {
    ...contextUsage,
    loadedSkills: {
      ...contextUsage.loadedSkills,
      skills: (contextUsage.loadedSkills?.skills ?? []).map(
        normalizeThreadSkillUsage,
      ),
    },
  };
}

function normalizeThreadTokenUsage(tokenUsage) {
  if (!tokenUsage || typeof tokenUsage !== "object") {
    return null;
  }

  return {
    ...tokenUsage,
    total: normalizeTokenUsageBreakdown(tokenUsage.total),
    last: normalizeTokenUsageBreakdown(tokenUsage.last),
    modelContextWindow:
      typeof tokenUsage.modelContextWindow === "number"
        ? tokenUsage.modelContextWindow
        : null,
  };
}

function normalizeTokenUsageBreakdown(usage) {
  return {
    totalTokens: Number(usage?.totalTokens ?? 0),
    inputTokens: Number(usage?.inputTokens ?? 0),
    cachedInputTokens: Number(usage?.cachedInputTokens ?? 0),
    outputTokens: Number(usage?.outputTokens ?? 0),
    reasoningOutputTokens: Number(usage?.reasoningOutputTokens ?? 0),
  };
}

function normalizeThreadSkillUsage(skill) {
  return {
    ...skill,
    kind: normalizeThreadSkillKind(skill?.kind),
  };
}

function normalizeAvailableSkill(skill) {
  return {
    name: skill?.name ?? "skill",
    path: skill?.path ?? "",
    kind: "all",
  };
}

function normalizeThreadGoal(goal) {
  return {
    threadId: goal?.threadId ?? goal?.thread_id ?? "",
    objective: goal?.objective ?? "",
    status: normalizeStatusValue(goal?.status),
    tokenBudget:
      goal?.tokenBudget === undefined ? goal?.token_budget ?? null : goal.tokenBudget,
    tokensUsed: Number(goal?.tokensUsed ?? goal?.tokens_used ?? 0),
    timeUsedSeconds: Number(
      goal?.timeUsedSeconds ?? goal?.time_used_seconds ?? 0,
    ),
    createdAt: Number(goal?.createdAt ?? goal?.created_at ?? 0),
    updatedAt: Number(goal?.updatedAt ?? goal?.updated_at ?? 0),
  };
}

function normalizeTurn(turn, options = {}) {
  return {
    ...turn,
    status: normalizeStatusValue(turn.status),
    items: (turn.items ?? []).map((item) => normalizeItem(item, options)),
  };
}

function normalizeItem(item, options = {}) {
  if (!item || typeof item !== "object") {
    return item;
  }

  switch (item.type) {
    case "contextCompaction": {
      const replacementHistory = Array.isArray(item.replacementHistory)
        ? item.replacementHistory
        : null;
      const hasReplacementHistory = replacementHistory !== null;
      return {
        ...item,
        replacementHistory:
          options.includeCompactReplacementHistory ? replacementHistory : null,
        replacementHistoryStatus: hasReplacementHistory
          ? replacementHistory.length > 0
            ? "available"
            : "empty"
          : "missing",
        replacementHistoryCount: replacementHistory?.length ?? null,
      };
    }
    case "userMessage":
      return {
        ...item,
        content: (item.content ?? []).map(normalizeUserInput),
      };
    case "commandExecution":
      return {
        ...item,
        status: normalizeStatusValue(item.status),
        cwd: normalizePathValue(item.cwd),
      };
    case "fileChange":
      return {
        ...item,
        status: normalizeStatusValue(item.status),
        changes: (item.changes ?? []).map((change) => ({
          ...change,
          kind: normalizePatchChangeKind(change.kind),
        })),
      };
    case "mcpToolCall":
    case "dynamicToolCall":
    case "eventDrivenToolCall":
    case "builtinToolCall":
      return {
        ...item,
        status: normalizeStatusValue(item.status),
      };
    case "collabAgentToolCall":
      return {
        ...item,
        status: normalizeStatusValue(item.status),
        agentsStates: Object.fromEntries(
          Object.entries(item.agentsStates ?? {}).map(([key, state]) => [
            key,
            normalizeCollabAgentState(state),
          ]),
        ),
      };
    case "collabAgentStatusUpdate":
      return {
        ...item,
        lifecycleStatus: normalizeCollabAgentState(
          item.lifecycleStatus ?? item.status,
        ),
      };
    case "imageView":
      return {
        ...item,
        path: normalizePathValue(item.path),
      };
    case "imageGeneration":
      return {
        ...item,
        savedPath: normalizePathValue(item.savedPath),
      };
    default:
      return item;
  }
}

function normalizeCollabAgentState(state) {
  return {
    ...state,
    lifecycleStatus: normalizeThreadLifecycleStatus(
      state?.lifecycleStatus ?? state?.status,
    ),
  };
}

function normalizeUserInput(input) {
  if (!input || typeof input !== "object") {
    return input;
  }

  if (
    input.type === "image" &&
    typeof input.url === "string" &&
    input.image_url == null
  ) {
    return {
      ...input,
      image_url: input.url,
    };
  }

  return input;
}

function normalizeStatusValue(status) {
  if (typeof status === "string") {
    return status;
  }
  if (status && typeof status === "object" && typeof status.type === "string") {
    return status.type;
  }
  return "unknown";
}

function normalizePatchChangeKind(kind) {
  if (typeof kind === "string") {
    return kind;
  }
  if (kind && typeof kind === "object" && typeof kind.type === "string") {
    return kind.type;
  }
  return "update";
}

function normalizeThreadSkillKind(kind) {
  if (typeof kind === "string") {
    return kind;
  }
  if (kind && typeof kind === "object" && typeof kind.type === "string") {
    return kind.type;
  }
  return "all";
}

function normalizePathValue(value) {
  if (typeof value === "string" || value == null) {
    return value;
  }
  if (typeof value === "object" && typeof value.path === "string") {
    return value.path;
  }
  return String(value);
}

function openLinkTarget(target) {
  if (typeof target !== "string" || !target.trim()) {
    throw new Error("Cannot open empty link target");
  }

  const normalizedTarget = target.trim();
  if (isLocalLinkTarget(normalizedTarget)) {
    const filePath = localFilePathFromTarget(
      normalizedTarget,
      defaultWorkspace,
    );
    return openLocalPath(filePath);
  }

  return shell.openExternal(normalizedTarget);
}

function openLocalPath(filePath) {
  return shell.openPath(filePath).then((result) => {
    if (result) {
      throw new Error(result);
    }
  });
}

async function readLocalFileTarget(target) {
  if (typeof target !== "string" || !target.trim()) {
    throw new Error("Cannot preview empty link target");
  }

  if (!isLocalLinkTarget(target.trim())) {
    throw new Error("Only local file links can be previewed");
  }

  const {
    line,
    column,
    path: filePath,
  } = parseLocalFileTarget(target.trim(), defaultWorkspace);
  const displayPath = path.relative(defaultWorkspace, filePath) || filePath;
  const extension = path.extname(filePath).toLowerCase();
  const imageMime = imageMimeForExtension(extension);
  const stat = await fs.stat(filePath);
  if (!stat.isFile()) {
    throw new Error("Only files can be previewed");
  }

  if (imageMime) {
    return {
      path: filePath,
      displayPath,
      content: "",
      language: "image",
      line: null,
      column: null,
      lsp: {
        enabled: false,
        languageId: null,
        lspStatus: { phase: "plain", detail: "Image preview" },
        serverLabel: null,
        workspaceRoot: null,
        reason: "Image file",
      },
      image: {
        path: filePath,
        mimeType: imageMime,
        name: path.basename(filePath),
        byteSize: stat.size,
      },
      pdf: null,
    };
  }

  const pdfPreviewToken = randomUUID();
  const pdf = buildPdfPreview(filePath, stat.size, pdfPreviewToken);
  if (pdf) {
    localFilePreviewTargetsByToken.set(pdfPreviewToken, {
      path: filePath,
      mimeType: pdf.mimeType,
    });
    return {
      path: filePath,
      displayPath,
      content: "",
      language: "pdf",
      line: null,
      column: null,
      lsp: {
        enabled: false,
        languageId: null,
        lspStatus: { phase: "plain", detail: "PDF preview" },
        serverLabel: null,
        workspaceRoot: null,
        reason: "PDF file",
      },
      image: null,
      pdf,
    };
  }

  const content = await fs.readFile(filePath, "utf8");
  const lsp = await lspManager.describeFile(filePath);

  return {
    path: filePath,
    displayPath,
    content,
    language: languageForFilePath(filePath),
    line,
    column,
    lsp,
    image: null,
    pdf: null,
  };
}

function registerLocalFilePreviewProtocol() {
  protocol.handle(FILE_PREVIEW_PROTOCOL, async (request) => {
    const target = localFilePreviewTargetForUrl(request.url);
    if (!target) {
      return new Response("Not found", { status: 404 });
    }

    try {
      return await net.fetch(pathToFileURL(target.path).href);
    } catch (error) {
      console.error(
        "[prototype] failed to serve local file preview",
        JSON.stringify({
          message: error instanceof Error ? error.message : String(error),
        }),
      );
      return new Response("Unable to load preview", { status: 500 });
    }
  });
}

function localFilePreviewTargetForUrl(url) {
  let parsed;
  try {
    parsed = new URL(url);
  } catch {
    return null;
  }
  if (
    parsed.protocol !== `${FILE_PREVIEW_PROTOCOL}:` ||
    parsed.hostname !== "pdf"
  ) {
    return null;
  }
  const token = decodeURIComponent(parsed.pathname.split("/").filter(Boolean)[0] ?? "");
  const target = localFilePreviewTargetsByToken.get(token);
  if (!target || target.mimeType !== "application/pdf") {
    return null;
  }
  return target;
}

async function readLocalImageTarget(target) {
  if (typeof target !== "string" || !target.trim()) {
    throw new Error("Cannot load empty image path");
  }
  const trimmed = target.trim();
  const filePath = path.isAbsolute(trimmed)
    ? trimmed
    : path.resolve(defaultWorkspace, trimmed);
  const mimeType = imageMimeForExtension(path.extname(filePath).toLowerCase());
  if (!mimeType) {
    throw new Error(`Unsupported image type for ${path.basename(filePath)}`);
  }
  const data = await fs.readFile(filePath);
  return {
    path: filePath,
    name: path.basename(filePath),
    mimeType,
    byteSize: data.length,
    bytes: Uint8Array.from(data).buffer,
  };
}

async function listLocalDirectoryTarget(target) {
  if (typeof target !== "string" || !target.trim()) {
    throw new Error("Cannot browse an empty directory path");
  }

  const trimmed = target.trim();
  const directoryPath = path.isAbsolute(trimmed)
    ? trimmed
    : path.resolve(defaultWorkspace, trimmed);
  const stat = await fs.stat(directoryPath);
  if (!stat.isDirectory()) {
    throw new Error("Only directories can be browsed");
  }

  const entries = await fs.readdir(directoryPath, { withFileTypes: true });
  return {
    path: directoryPath,
    entries: entries
      .filter((entry) => entry.isDirectory() || entry.isFile())
      .map((entry) => ({
        path: path.join(directoryPath, entry.name),
        name: entry.name,
        kind: entry.isDirectory() ? "directory" : "file",
      }))
      .sort(
        (left, right) =>
          Number(left.kind === "file") - Number(right.kind === "file") ||
          left.name.localeCompare(right.name),
      ),
  };
}

function imageMimeForExtension(extension) {
  switch (extension) {
    case ".png":
      return "image/png";
    case ".jpg":
    case ".jpeg":
      return "image/jpeg";
    case ".gif":
      return "image/gif";
    case ".webp":
      return "image/webp";
    case ".bmp":
      return "image/bmp";
    case ".svg":
      return "image/svg+xml";
    case ".avif":
      return "image/avif";
    case ".heic":
    case ".heif":
      return "image/heic";
    default:
      return null;
  }
}

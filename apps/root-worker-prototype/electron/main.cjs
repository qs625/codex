const path = require("node:path");
const fs = require("node:fs/promises");
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
  WebContentsView,
} = require("electron");
const { AppServerClient } = require("./appServerClient.cjs");
const {
  browserNavigationEventDecision,
  normalizeBrowserTarget,
} = require("./browserPanelSecurity.cjs");
const {
  browserPanelWebPreferences,
  browserSessionPartition,
} = require("./browserPanelConfig.cjs");
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
const { readGitCommitFiles, readGitSnapshot } = require("./gitPanel.cjs");
const { LspManager } = require("./lsp/manager.cjs");
const {
  normalizeThreadLifecycleStatus,
} = require("./threadLifecycleStatus.cjs");
const { normalizeThreadSnapshot } = require("./threadSnapshots.cjs");
const {
  buildChatCompatCwd,
  buildCreateThreadStartParams,
  buildSubscribeThreadResumeParams,
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
  shouldNotifyRuntimeRestartErrorOnSelf,
} = require("./restartRecoveryPrompts.cjs");
const {
  notifyRecoverableRestartErrorOnSelf,
} = require("./restartRecoverySelfNotice.cjs");
const { createRuntimeLauncher } = require("./runtimeLauncher.cjs");
const {
  recoverPayloadRuntimeFailureIfPresent,
  recordLauncherRecoveryIfPresent,
  shouldRecordLauncherRecovery,
  writePayloadFailureEvidence,
} = require("./runtimeLaunchState.cjs");
const { applyRemoteDebuggingConfig } = require("./remoteDebugging.cjs");

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
    appExit: () => app.quit(),
    cleanupPreparedArtifact: (preparedRoot) =>
      removeInstalledArtifactTree(preparedRoot),
    resolvePlan: () => resolveInstalledArtifactUpdatePlanInWorker(),
    runtimeLauncher,
    updateArtifacts: (plan) => updateInstalledArtifactsInWorker(plan),
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
let browserPanelTabCounter = 0;
const threadRuntimeById = new Map();
const localFilePreviewTargetsByToken = new Map();
let autoResumeCoordinator = null;
let runtimeRestartController = null;
let runtimeRestartIntentStore = null;
let quittingAfterAppServerStop = false;
let fatalPayloadExitRequested = false;
const defaultWorkspace = resolveDefaultWorkspace();
const devServerUrl =
  process.env.ROOT_WORKER_DEV_SERVER_URL ?? "http://127.0.0.1:5173";

applyRemoteDebuggingConfig(app, process.env, console);

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
  const listResult = await listThreads(defaultWorkspace);
  const threads = listResult.threads;
  const expectedRestart =
    await getRuntimeRestartController().recoverPending();
  const autoResume =
    await getAutoResumeCoordinator().runAfterRuntimeRestartRecovery({
      threads,
      expectedRestart,
    });
  return {
    workspace: defaultWorkspace,
    threads,
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
  configurePermissionHandlers(session.defaultSession, ({ webContents, permission }) =>
    permission === "media" && !isBrowserPanelWebContents(webContents),
  );
  configurePermissionHandlers(
    session.fromPartition(browserSessionPartition),
    () => false,
  );
  void ensureDefaultWorkspace()
    .then(async () => {
      await appServerClient.ready();
      const payloadRecovery = await recoverPayloadRuntimeFailureIfPresent({
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
      }).catch((error) => {
        console.error(
          "[prototype] payload recovery input failed; evidence retained",
          error,
        );
        return {
          evidence: true,
          payloadEvidence: error?.payloadEvidence === true,
          recovered: false,
        };
      });
      if (shouldRecordLauncherRecovery(payloadRecovery)) {
        await recordLauncherRecoveryIfPresent({
          appServerClient,
          evidencePath: process.env.RUNTIME_CAPSULE_FAILURE_EVIDENCE_PATH,
          fs,
          listThreads: () => listThreads(defaultWorkspace),
          subscribeThread,
        });
      }
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

function browserPanelForWindow(window) {
  const existing = browserPanelsByWindowId.get(window.id);
  if (existing) {
    return existing;
  }

  if (typeof WebContentsView !== "function") {
    throw new Error("This Electron version does not support WebContentsView");
  }

  const panel = {
    window,
    visible: false,
    bounds: normalizeBrowserBounds(null),
    tabs: [],
    activeTabId: null,
    attachedTabId: null,
    destroying: false,
  };

  browserPanelsByWindowId.set(window.id, panel);
  createBrowserPanelTab(panel, { activate: true });
  return panel;
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
  panel.bounds = normalizeBrowserBounds(bounds);
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

function normalizeBrowserBounds(bounds) {
  return {
    x: Math.max(0, Math.round(Number(bounds?.x) || 0)),
    y: Math.max(0, Math.round(Number(bounds?.y) || 0)),
    width: Math.max(0, Math.round(Number(bounds?.width) || 0)),
    height: Math.max(0, Math.round(Number(bounds?.height) || 0)),
  };
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
  const decision = browserNavigationEventDecision(event, target);
  if (decision.allow) {
    return;
  }

  event.preventDefault();
  tab.state.error = decision.reason;
  tab.state.loading = false;
  sendBrowserPanelState(panel);
}

function configurePermissionHandlers(targetSession, isAllowed) {
  targetSession.setPermissionCheckHandler((webContents, permission) => {
    return isAllowed({ webContents, permission });
  });
  targetSession.setPermissionRequestHandler(
    (webContents, permission, callback) => {
      callback(isAllowed({ webContents, permission }));
    },
  );
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
      execute: (notification) =>
        observeClientRelaunchResult(
          handleClientRelaunchNotification(notification),
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
  if (!shouldNotifyRuntimeRestartErrorOnSelf(record)) {
    return;
  }
  return notifyRecoverableRestartErrorOnSelf({
    sourceThreadId: record.requestedByThreadId,
    prompt: expectedRuntimeRestartRecoveryPrompt(record),
    listThreads: () => listThreads(defaultWorkspace),
    readThread,
    subscribeThread,
    sendUserInput: (thread, text) =>
      startThreadTurn({
        threadId: thread?.id,
        model: thread?.model ?? null,
        modelProvider: thread?.modelProvider ?? null,
        effort: thread?.reasoningEffort ?? null,
        text,
        skills: [],
        images: [],
      }),
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
  const resume = await appServerClient.request(
    "thread/resume",
    buildSubscribeThreadResumeParams(threadId),
  );
  const existingRuntime = threadRuntimeById.get(threadId) ?? null;
  const runtime = resolveRuntimeForResume(existingRuntime, resume);
  rememberThreadRuntime(threadId, runtime);
  return {
    thread: resume.thread
      ? normalizeThread({ ...resume.thread, turns: [] }, runtime)
      : null,
  };
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

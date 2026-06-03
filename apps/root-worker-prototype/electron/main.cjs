const path = require("node:path");
const fs = require("node:fs/promises");
const {
  app,
  BrowserWindow,
  ipcMain,
  session,
  shell,
  systemPreferences,
} = require("electron");
const { AppServerClient } = require("./appServerClient.cjs");
const {
  isLocalLinkTarget,
  localFilePathFromTarget,
  parseLocalFileTarget,
} = require("./fileTargets.cjs");
const { LspManager } = require("./lsp/manager.cjs");
const {
  mergeThreadSnapshots,
  normalizeThreadSnapshot,
} = require("./threadSnapshots.cjs");
const { withRealtimeConversationFeature } = require("./threadConfig.cjs");
const { buildTurnInput } = require("./turnInput.cjs");
const {
  ensureDefaultWorkspace,
  resolveDefaultWorkspace,
} = require("./workspace.cjs");

const rendererMode = process.env.ROOT_WORKER_RENDERER_MODE ?? "built";
const isDev = rendererMode === "dev";
const appServerClient = new AppServerClient();
const lspManager = new LspManager();
const windows = new Set();
const defaultWorkspace = resolveDefaultWorkspace();
const devServerUrl = "http://127.0.0.1:5173";
const builtRendererPath = path.join(__dirname, "../dist/index.html");

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
    windows.delete(window);
  });

  if (isDev) {
    void window.loadURL(devServerUrl);
    window.webContents.openDevTools({ mode: "detach" });
  } else {
    await ensureBuiltRenderer();
    void window.loadFile(builtRendererPath);
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
  broadcast("codex:notification", normalizeNotification(notification));
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

ipcMain.handle("codex:bootstrap", async () => {
  await ensureDefaultWorkspace();
  const threads = await listThreads(defaultWorkspace);
  return {
    workspace: defaultWorkspace,
    threads,
    appServer: appServerClient.status,
  };
});

ipcMain.handle("codex:listThreads", async (_event, cwd = defaultWorkspace) => {
  await ensureDefaultWorkspace();
  return { data: await listThreads(cwd) };
});

ipcMain.handle("codex:listSkills", async (_event, cwd = defaultWorkspace) => {
  await ensureDefaultWorkspace();
  return listSkills(cwd);
});

ipcMain.handle("codex:createThread", async (_event, payload) => {
  await ensureDefaultWorkspace();
  const params = withRealtimeConversationFeature({
    cwd: payload?.cwd ?? defaultWorkspace,
    approvalPolicy: "never",
    sandbox: "danger-full-access",
    threadSource: "user",
  });
  const start = await appServerClient.request("thread/start", params);

  if (payload?.name && payload.name.trim()) {
    await appServerClient.request("thread/name/set", {
      threadId: start.thread.id,
      name: payload.name.trim(),
    });
  }

  return readThread(start.thread.id, true, {
    model: start.model ?? null,
    reasoningEffort: start.reasoningEffort ?? null,
  });
});

ipcMain.handle("codex:archiveThread", async (_event, threadId) => {
  await appServerClient.request("thread/archive", { threadId });
  return { ok: true };
});

ipcMain.handle(
  "codex:readThread",
  async (_event, threadId, subscribe = true) => {
    let runtime = null;
    let resumedThread = null;
    if (subscribe) {
      const resume = await appServerClient.request(
        "thread/resume",
        withRealtimeConversationFeature({ threadId }),
      );
      runtime = {
        model: resume.model ?? null,
        reasoningEffort: resume.reasoningEffort ?? null,
      };
      resumedThread = resume.thread
        ? normalizeThread(resume.thread, runtime)
        : null;
    }
    const payload = await readThread(threadId, true, runtime);
    return {
      thread: mergeThreadSnapshots(resumedThread, payload.thread),
    };
  },
);

ipcMain.handle("codex:openLink", async (_event, target) => {
  await openLinkTarget(target);
  return { ok: true };
});

ipcMain.handle("codex:readLocalFile", async (_event, target) => {
  return readLocalFileTarget(target);
});

ipcMain.handle("codex:readLocalImage", async (_event, target) => {
  return readLocalImageTarget(target);
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

  return appServerClient.request("turn/start", {
    threadId: payload.threadId,
    input,
  });
});

ipcMain.handle("codex:interruptTurn", async (_event, payload) => {
  return appServerClient.request("turn/interrupt", {
    threadId: payload.threadId,
    turnId: payload.turnId,
  });
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
  session.defaultSession.setPermissionCheckHandler(
    (_webContents, permission) => {
      return permission === "media";
    },
  );
  session.defaultSession.setPermissionRequestHandler(
    (_webContents, permission, callback) => {
      if (permission === "media") {
        callback(true);
        return;
      }
      callback(false);
    },
  );
  void ensureDefaultWorkspace()
    .then(async () => {
      await primeMicrophoneAccessPrompt();
      return createWindow();
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

async function ensureBuiltRenderer() {
  try {
    await fs.access(builtRendererPath);
  } catch (error) {
    throw new Error(
      `Built renderer not found at ${builtRendererPath}. Run 'pnpm --filter @my-codex/root-worker-prototype build' before 'pnpm --filter @my-codex/root-worker-prototype start'.`,
      { cause: error },
    );
  }
}

function handleStartupError(error) {
  console.error("[prototype] failed to start renderer", error);
  app.exit(1);
}

async function listThreads(cwd) {
  const response = await appServerClient.request("thread/list", {
    limit: 200,
    sourceKinds: [
      "appServer",
      "cli",
      "vscode",
      "exec",
      "subAgent",
      "subAgentReview",
      "subAgentCompact",
      "subAgentThreadSpawn",
      "subAgentOther",
      "unknown",
    ],
  });
  return response.data.map(normalizeThread);
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

async function readThread(threadId, includeTurns, runtime = null) {
  const response = await appServerClient.request("thread/read", {
    threadId,
    includeTurns,
  });
  return { thread: normalizeThread(response.thread, runtime) };
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
        status: normalizeThreadStatus(notification.params.status),
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

function normalizeThread(thread, runtime = null) {
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
    reasoningEffort: runtime?.reasoningEffort ?? thread.reasoningEffort ?? null,
    status: normalizeThreadStatus(thread.status),
    skills: (thread.skills ?? []).map(normalizeThreadSkill),
    threadUsage,
    ...(Object.prototype.hasOwnProperty.call(thread, "tokenUsage")
      ? { tokenUsage }
      : {}),
    ...(Object.prototype.hasOwnProperty.call(thread, "contextUsage")
      ? { contextUsage }
      : {}),
    turns: (thread.turns ?? []).map(normalizeTurn),
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

function normalizeTurn(turn) {
  return {
    ...turn,
    status: normalizeStatusValue(turn.status),
    items: (turn.items ?? []).map(normalizeItem),
  };
}

function normalizeItem(item) {
  if (!item || typeof item !== "object") {
    return item;
  }

  switch (item.type) {
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
    case "collabAgentToolCall":
      return {
        ...item,
        status: normalizeStatusValue(item.status),
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

function normalizeThreadStatus(status) {
  if (status && typeof status === "object" && typeof status.type === "string") {
    return {
      type: status.type,
      ...(status.type === "active"
        ? {
            activeFlags: Array.isArray(status.activeFlags)
              ? status.activeFlags.filter((flag) => typeof flag === "string")
              : [],
          }
        : {}),
    };
  }

  if (typeof status === "string") {
    return { type: status };
  }

  return { type: "notLoaded" };
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

  if (imageMime) {
    const { size } = await fs.stat(filePath);
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
        byteSize: size,
      },
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
  };
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

function languageForFilePath(filePath) {
  const extension = path.extname(filePath).toLowerCase();
  switch (extension) {
    case ".cjs":
    case ".js":
    case ".jsx":
    case ".mjs":
      return "javascript";
    case ".ts":
    case ".tsx":
      return "typescript";
    case ".rs":
      return "rust";
    case ".json":
      return "json";
    case ".md":
      return "markdown";
    case ".css":
      return "css";
    case ".html":
      return "html";
    case ".yml":
    case ".yaml":
      return "yaml";
    case ".sh":
    case ".zsh":
    case ".fish":
      return "shell";
    default:
      return "plaintext";
  }
}

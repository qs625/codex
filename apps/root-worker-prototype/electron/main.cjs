const path = require("node:path");
const os = require("node:os");
const fs = require("node:fs/promises");
const { fileURLToPath } = require("node:url");
const { app, BrowserWindow, ipcMain, shell } = require("electron");
const { AppServerClient } = require("./appServerClient.cjs");
const { LspManager } = require("./lsp/manager.cjs");

const isDev = !app.isPackaged;
const appServerClient = new AppServerClient();
const lspManager = new LspManager();
const windows = new Set();
const defaultWorkspace = process.env.ROOT_WORKER_WORKSPACE ?? path.resolve(process.cwd(), "../..");
const devServerUrl = "http://127.0.0.1:5173";

function createWindow() {
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
    void window.loadFile(path.join(__dirname, "../dist/index.html"));
  }

  return window;
}

function broadcast(channel, payload) {
  for (const window of windows) {
    if (!window.isDestroyed()) {
      window.webContents.send(channel, payload);
    }
  }
}

appServerClient.on("notification", (notification) => {
  broadcast("codex:notification", normalizeNotification(notification));
});

appServerClient.on("status", (status) => {
  broadcast("codex:status", status);
});

ipcMain.handle("codex:health", async () => {
  await appServerClient.ready();
  return {
    ok: true,
    appServer: appServerClient.status,
    workspace: defaultWorkspace,
  };
});

ipcMain.handle("codex:bootstrap", async () => {
  const threads = await listThreads(defaultWorkspace);
  return {
    workspace: defaultWorkspace,
    threads,
    appServer: appServerClient.status,
  };
});

ipcMain.handle("codex:listThreads", async (_event, cwd = defaultWorkspace) => {
  return { data: await listThreads(cwd) };
});

ipcMain.handle("codex:createThread", async (_event, payload) => {
  const start = await appServerClient.request("thread/start", {
    cwd: payload?.cwd ?? defaultWorkspace,
    approvalPolicy: "never",
    sandbox: "workspace-write",
    threadSource: "user",
  });

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

ipcMain.handle("codex:readThread", async (_event, threadId, subscribe = true) => {
  let runtime = null;
  if (subscribe) {
    const resume = await appServerClient.request("thread/resume", { threadId });
    runtime = {
      model: resume.model ?? null,
      reasoningEffort: resume.reasoningEffort ?? null,
    };
  }
  return readThread(threadId, true, runtime);
});

ipcMain.handle("codex:openLink", async (_event, target) => {
  await openLinkTarget(target);
  return { ok: true };
});

ipcMain.handle("codex:readLocalFile", async (_event, target) => {
  return readLocalFileTarget(target);
});

ipcMain.handle("codex:lspDefinition", async (_event, payload) => {
  return lspManager.definition({
    filePath: payload.path,
    line: payload.line,
    column: payload.column,
  });
});

ipcMain.handle("codex:sendMessage", async (_event, payload) => {
  const input = [];

  if (payload.text.trim()) {
    input.push({
      type: "text",
      text: payload.text.trim(),
      text_elements: [],
    });
  }

  for (const image of payload.images ?? []) {
    if (!image?.dataUrl) {
      continue;
    }
    input.push({
      type: "image",
      image_url: image.dataUrl,
    });
  }

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

app.whenReady().then(() => {
  createWindow();
  app.on("activate", () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      createWindow();
    }
  });
});

app.on("window-all-closed", () => {
  if (process.platform !== "darwin") {
    app.quit();
  }
});

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

  if (notification.method === "turn/started" || notification.method === "turn/completed") {
    return {
      ...notification,
      params: {
        ...notification.params,
        turn: normalizeTurn(notification.params.turn),
      },
    };
  }

  if (notification.method === "item/started" || notification.method === "item/completed") {
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
  return {
    ...thread,
    model: runtime?.model ?? thread.model ?? null,
    reasoningEffort: runtime?.reasoningEffort ?? thread.reasoningEffort ?? null,
    status: normalizeStatusValue(thread.status),
    turns: (thread.turns ?? []).map(normalizeTurn),
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

  if (input.type === "image" && typeof input.url === "string" && input.image_url == null) {
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
    const filePath = localFilePathFromTarget(normalizedTarget);
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

function isLocalLinkTarget(target) {
  return (
    target.startsWith("file://") ||
    target.startsWith("/") ||
    target.startsWith("~/") ||
    target.startsWith("./") ||
    target.startsWith("../") ||
    target.startsWith("\\\\") ||
    /^[A-Za-z]:[\\/]/.test(target)
  );
}

function localFilePathFromTarget(target) {
  if (target.startsWith("file://")) {
    const url = new URL(target);
    return fileURLToPath(url);
  }

  const withoutHash = target.split("#", 1)[0];
  const withoutLocationSuffix = withoutHash.replace(
    /:\d+(?::\d+)?(?:-\d+(?::\d+)?)?$/,
    "",
  );

  if (withoutLocationSuffix.startsWith("~/")) {
    return path.join(os.homedir(), withoutLocationSuffix.slice(2));
  }

  if (
    withoutLocationSuffix.startsWith("./") ||
    withoutLocationSuffix.startsWith("../")
  ) {
    return path.resolve(defaultWorkspace, withoutLocationSuffix);
  }

  return withoutLocationSuffix;
}

async function readLocalFileTarget(target) {
  if (typeof target !== "string" || !target.trim()) {
    throw new Error("Cannot preview empty link target");
  }

  if (!isLocalLinkTarget(target.trim())) {
    throw new Error("Only local file links can be previewed");
  }

  const { line, path: filePath } = parseLocalFileTarget(target.trim());
  const content = await fs.readFile(filePath, "utf8");
  const lsp = await lspManager.describeFile(filePath);

  return {
    path: filePath,
    displayPath: path.relative(defaultWorkspace, filePath) || filePath,
    content,
    language: languageForFilePath(filePath),
    line,
    lsp,
  };
}

function parseLocalFileTarget(target) {
  const normalizedPath = localFilePathFromTarget(target);
  const lineMatch = normalizedPath.match(/:(\d+)(?::\d+)?$/);

  if (!lineMatch) {
    return { path: normalizedPath, line: null };
  }

  const possiblePath = normalizedPath.slice(0, Math.max(0, normalizedPath.length - lineMatch[0].length));
  if (!possiblePath || !path.extname(possiblePath)) {
    return { path: normalizedPath, line: null };
  }

  return {
    path: possiblePath,
    line: Number.parseInt(lineMatch[1], 10),
  };
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

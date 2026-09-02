const { spawn } = require("node:child_process");
const crypto = require("node:crypto");
const fs = require("node:fs");
const net = require("node:net");
const os = require("node:os");
const path = require("node:path");
const { createInterface } = require("node:readline");
const { EventEmitter } = require("node:events");
const { buildDesktopEnvironment } = require("./environment.cjs");
const {
  ensureDefaultWorkspaceSync,
  ensureMorpheusSourceInstructionSync,
  ensureWorkspaceExistsSync,
} = require("./workspace.cjs");
const { ensureSelfProjectSync } = require("./selfProject.cjs");

const DEFAULT_MOBILE_LISTEN_URL = "ws://0.0.0.0:8910";
const MOBILE_LISTEN_PORT_FALLBACK_ATTEMPTS = 20;
const MOBILE_CONNECTION_REFRESH_INTERVAL_MS = 5_000;
const PACKAGED_APP_SERVER_FILE_NAME =
  process.platform === "win32" ? "app-server.exe" : "app-server";
const PACKAGED_APP_SERVER_RELATIVE_PATH = path.join(
  "bin",
  PACKAGED_APP_SERVER_FILE_NAME,
);
const PACKAGED_COMPACT_PROMPT_RELATIVE_PATH = path.join(
  "default-config",
  "compact",
  "COMPACT.md",
);
const HOME_COMPACT_PROMPT_RELATIVE_PATH = path.join("compact", "COMPACT.md");

class AppServerClient extends EventEmitter {
  constructor() {
    super();
    this.child = null;
    this.pending = new Map();
    this.nextRequestId = 1;
    this.mobileConnection = {
      enabled: false,
      reason: "app-server is starting",
    };
    this.mobileConnectionRefreshEnv = {};
    this.mobileConnectionRefreshTimer = null;
    this.readyPromise = new Promise((resolve, reject) => {
      this.readyResolve = resolve;
      this.readyReject = reject;
    });
    void this.start().catch((error) => {
      this.readyReject(error instanceof Error ? error : new Error(String(error)));
    });
  }

  stopMobileConnectionRefresh() {
    if (this.mobileConnectionRefreshTimer) {
      clearInterval(this.mobileConnectionRefreshTimer);
      this.mobileConnectionRefreshTimer = null;
    }
  }

  startMobileConnectionRefresh() {
    this.stopMobileConnectionRefresh();
    if (!this.mobileConnection.enabled) {
      return;
    }
    if (this.mobileConnectionRefreshEnv.ROOT_WORKER_MOBILE_ENDPOINT) {
      return;
    }
    const tick = () => this.refreshMobileConnectionInfo();
    this.mobileConnectionRefreshTimer = setInterval(
      tick,
      MOBILE_CONNECTION_REFRESH_INTERVAL_MS,
    );
    this.mobileConnectionRefreshTimer.unref?.();
  }

  refreshMobileConnectionInfo(options = {}) {
    const next = refreshMobileConnectionInfo(
      this.mobileConnection,
      this.mobileConnectionRefreshEnv,
      options,
    );
    if (mobileConnectionInfoEqual(next, this.mobileConnection)) {
      return false;
    }
    this.mobileConnection = next;
    this.emit("status", this.status);
    return true;
  }

  async ready() {
    return this.readyPromise;
  }

  async request(method, params) {
    if (method !== "initialize") {
      await this.ready();
    }
    return this.sendRequest(method, params);
  }

  async notify(method, params) {
    if (method !== "initialized") {
      await this.ready();
    }
    this.write({ method, params });
  }

  async respondToServerRequest(id, result) {
    await this.ready();
    this.write({ id, result });
  }

  async rejectServerRequest(id, message, code = -32601) {
    await this.ready();
    this.write({
      id,
      error: {
        code,
        message,
      },
    });
  }

  get status() {
    return {
      connected: this.child?.exitCode == null,
      pid: this.child?.pid ?? null,
      mobileConnection: this.mobileConnection,
    };
  }

  getMobileConnectionInfo() {
    return this.mobileConnection;
  }

  async start() {
    const env = buildAppServerEnvironment(process.env);
    const morpheusHome = env.MORPHEUS_HOME;
    ensureMorpheusHomeDefaults(morpheusHome, { warn: console.warn });
    const appServerCwd = prepareAppServerWorkspace(env, {
      cwd: process.cwd(),
      warn: console.warn,
    });
    const launch = await resolveAppServerLaunch(process.env, {
      morpheusHome,
      randomToken: () => crypto.randomBytes(32).toString("base64url"),
      writeTokenFile,
      checkListenAvailable: canBindWebSocketListenUrl,
    });
    const command = launch.command;
    this.mobileConnection = launch.mobileConnection;
    this.mobileConnectionRefreshEnv = launch.mobileConnectionEnv ?? {};
    this.child = spawn(command, {
      cwd: appServerCwd,
      env,
      shell: true,
      stdio: "pipe",
    });

    this.child.stderr.on("data", (chunk) => {
      process.stderr.write(chunk);
    });

    this.child.on("exit", (code, signal) => {
      this.stopMobileConnectionRefresh();
      const reason = `app-server exited (${code ?? "null"} / ${signal ?? "null"})`;
      const error = new Error(reason);
      for (const pending of this.pending.values()) {
        pending.reject(error);
      }
      this.pending.clear();
      this.emit("status", { connected: false, reason });
    });

    const stdout = createInterface({ input: this.child.stdout });
    stdout.on("line", (line) => {
      if (!line.trim()) {
        return;
      }
      try {
        this.handleMessage(JSON.parse(line));
      } catch (error) {
        console.error("Failed to parse app-server message", line, error);
      }
    });

    void this.initialize();
    this.startMobileConnectionRefresh();
  }

  async initialize() {
    try {
      const initializeResult = await this.request("initialize", {
        clientInfo: {
          name: "root_worker_prototype_electron",
          title: "Root Worker Prototype",
          version: "0.1.0",
        },
        capabilities: {
          experimentalApi: true,
        },
      });
      await this.notify("initialized", {});
      this.readyResolve();
      this.emit("status", { ...this.status, initializeResult });
    } catch (error) {
      this.readyReject(
        error instanceof Error ? error : new Error(String(error)),
      );
    }
  }

  write(message) {
    if (!this.child) {
      throw new Error("app-server process is not running");
    }
    this.child.stdin.write(`${JSON.stringify(message)}\n`);
  }

  sendRequest(method, params) {
    const id = this.nextRequestId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.write({ id, method, params });
    });
  }

  handleMessage(message) {
    if (!message || typeof message !== "object") {
      return;
    }

    if (
      (typeof message.id === "number" || typeof message.id === "string") &&
      typeof message.method === "string"
    ) {
      this.emit("request", {
        id: message.id,
        method: message.method,
        params: message.params,
      });
      return;
    }

    if (
      typeof message.id === "number" &&
      Object.prototype.hasOwnProperty.call(message, "result")
    ) {
      const pending = this.pending.get(message.id);
      if (!pending) {
        return;
      }
      this.pending.delete(message.id);
      pending.resolve(message.result);
      return;
    }

    if (
      typeof message.id === "number" &&
      Object.prototype.hasOwnProperty.call(message, "error")
    ) {
      const pending = this.pending.get(message.id);
      if (!pending) {
        return;
      }
      this.pending.delete(message.id);
      const errorPayload = message.error ?? {};
      pending.reject(
        new Error(
          `app-server request failed (${errorPayload.code ?? "unknown"}): ${errorPayload.message ?? "unknown error"}`,
        ),
      );
      return;
    }

    if (typeof message.method === "string") {
      this.emit("notification", {
        method: message.method,
        params: message.params,
      });
    }
  }
}

module.exports = {
  AppServerClient,
  buildAppServerEnvironment,
  buildDefaultAppServerCommand,
  buildMobileConnectionLaunch,
  ensureMorpheusHomeDefaults,
  findDefaultCompactPromptSeedPath,
  findPackagedAppServerBinaryPath,
  prepareAppServerWorkspace,
  resolveDefaultAppServerBinary,
  resolveAppServerCommand,
  resolveAppServerLaunch,
  refreshMobileConnectionInfo,
  resolveLanEndpoint,
  writeTokenFile,
};

function buildAppServerEnvironment(baseEnv = process.env, environmentOptions = {}) {
  const home = baseEnv.HOME ?? environmentOptions.home ?? os.homedir();
  const env = buildDesktopEnvironment(
    {
      ...baseEnv,
      HOME: home,
    },
    {
      ...environmentOptions,
      home,
    },
  );
  env.HOME = home;
  env.MORPHEUS_HOME = baseEnv.MORPHEUS_HOME ?? resolvePrototypeMorpheusHome(env);
  return env;
}

function resolveAppServerCommand(baseEnv = process.env) {
  return (
    baseEnv.APP_SERVER_CMD ??
    baseEnv.CODEX_APP_SERVER_CMD ??
    `${resolveDefaultAppServerBinary()} --listen stdio://`
  );
}

function prepareAppServerWorkspace(env, options = {}) {
  let appServerCwd = options.cwd ?? process.cwd();
  const packagedBinary = findPackagedAppServerBinaryPath(options);
  if (!packagedBinary) {
    return appServerCwd;
  }
  if (env.ROOT_WORKER_WORKSPACE) {
    ensureWorkspaceExistsSync(env.ROOT_WORKER_WORKSPACE);
    ensureMorpheusSourceInstructionSync(env, env.ROOT_WORKER_WORKSPACE, options);
    ensureSelfProjectSync(env, env.ROOT_WORKER_WORKSPACE, options);
    return env.ROOT_WORKER_WORKSPACE;
  }
  if (!env.ROOT_WORKER_WORKSPACE) {
    const defaultWorkspace = ensureDefaultWorkspaceSync(env, {
      ...options,
      isPackagedApp: true,
    });
    env.ROOT_WORKER_WORKSPACE = defaultWorkspace;
    ensureSelfProjectSync(env, defaultWorkspace, options);
    appServerCwd = defaultWorkspace;
  }
  return appServerCwd;
}

async function resolveAppServerLaunch(baseEnv = process.env, options = {}) {
  const customCommand = baseEnv.APP_SERVER_CMD ?? baseEnv.CODEX_APP_SERVER_CMD;
  if (customCommand) {
    return {
      command: customCommand,
      mobileConnection: {
        enabled: false,
        reason:
          "Mobile listener is unavailable when APP_SERVER_CMD overrides app-server launch.",
      },
      mobileConnectionEnv: captureMobileConnectionEnv(baseEnv),
    };
  }
  const mobileLaunch = await buildMobileConnectionLaunch(baseEnv, options);
  return {
    command: buildDefaultAppServerCommand({
      appServerBinary: resolveDefaultAppServerBinary(options),
      mobileLaunch,
    }),
    mobileConnection: mobileLaunch.info,
    mobileConnectionEnv: captureMobileConnectionEnv(baseEnv),
  };
}

function buildDefaultAppServerCommand({ appServerBinary, mobileLaunch }) {
  const base = `${appServerBinary} --listen stdio://`;
  if (!mobileLaunch.enabled) {
    return base;
  }
  return [
    base,
    "--mobile-listen",
    shellQuote(mobileLaunch.listenUrl),
    "--ws-auth capability-token",
    "--ws-token-file",
    shellQuote(mobileLaunch.tokenFile),
  ].join(" ");
}

async function buildMobileConnectionLaunch(baseEnv = process.env, options = {}) {
  const listenUrl = baseEnv.ROOT_WORKER_MOBILE_LISTEN ?? DEFAULT_MOBILE_LISTEN_URL;
  if (listenUrl === "off") {
    return {
      enabled: false,
      info: {
        enabled: false,
        reason: "Mobile listener is disabled by ROOT_WORKER_MOBILE_LISTEN=off.",
      },
    };
  }
  const parsedListenUrl = parseMobileWebSocketListenUrl(listenUrl);
  if (!parsedListenUrl) {
    return {
      enabled: false,
      info: {
        enabled: false,
        reason: "ROOT_WORKER_MOBILE_LISTEN must be a ws://IP:PORT bind URL.",
      },
    };
  }
  const chosenListenUrl = await resolveAvailableMobileListenUrl(
    listenUrl,
    parsedListenUrl,
    baseEnv,
    options,
  );
  if (!chosenListenUrl) {
    return {
      enabled: false,
      info: {
        enabled: false,
        reason: buildUnavailableMobileListenReason(listenUrl, baseEnv),
      },
    };
  }
  const token = baseEnv.ROOT_WORKER_MOBILE_TOKEN ?? options.randomToken?.();
  if (!token) {
    return {
      enabled: false,
      info: {
        enabled: false,
        reason: "Mobile listener token generation failed.",
      },
    };
  }
  const morpheusHome = options.morpheusHome ?? resolvePrototypeMorpheusHome(baseEnv);
  const tokenFile =
    options.tokenFile ??
    path.join(morpheusHome, "root-worker-mobile-ws-token");
  options.writeTokenFile?.(tokenFile, token);
  const endpoint = resolveLanEndpoint(chosenListenUrl, baseEnv, options);
  return {
    enabled: true,
    listenUrl: chosenListenUrl,
    token,
    tokenFile,
    info: {
      enabled: true,
      bindEndpoint: chosenListenUrl,
      endpoint,
      token,
      auth: "capability-token",
    },
  };
}

async function resolveAvailableMobileListenUrl(
  listenUrl,
  parsedListenUrl,
  baseEnv,
  options,
) {
  if (!options.checkListenAvailable) {
    return listenUrl;
  }
  if (await options.checkListenAvailable(listenUrl)) {
    return listenUrl;
  }
  if (baseEnv.ROOT_WORKER_MOBILE_ENDPOINT) {
    return null;
  }
  for (
    let port = parsedListenUrl.port + 1, attempts = 0;
    port <= 65535 && attempts < MOBILE_LISTEN_PORT_FALLBACK_ATTEMPTS;
    port += 1, attempts += 1
  ) {
    const candidate = formatMobileWebSocketListenUrl(parsedListenUrl.host, port);
    if (await options.checkListenAvailable(candidate)) {
      return candidate;
    }
  }
  return null;
}

function buildUnavailableMobileListenReason(listenUrl, baseEnv) {
  if (baseEnv.ROOT_WORKER_MOBILE_ENDPOINT) {
    return `Mobile listener bind address is unavailable: ${listenUrl}. Automatic port fallback is disabled because ROOT_WORKER_MOBILE_ENDPOINT is set.`;
  }
  return `Mobile listener bind address is unavailable: ${listenUrl}. No fallback port was available.`;
}

function captureMobileConnectionEnv(baseEnv = process.env) {
  return {
    ROOT_WORKER_MOBILE_ENDPOINT: baseEnv.ROOT_WORKER_MOBILE_ENDPOINT,
  };
}

function refreshMobileConnectionInfo(info, baseEnv = process.env, options = {}) {
  if (!info?.enabled) {
    return info;
  }
  const endpoint = resolveLanEndpoint(info.bindEndpoint, baseEnv, options);
  if (endpoint === info.endpoint) {
    return info;
  }
  return {
    ...info,
    endpoint,
  };
}

function resolveLanEndpoint(listenUrl, baseEnv = process.env, options = {}) {
  const override = baseEnv.ROOT_WORKER_MOBILE_ENDPOINT;
  if (override) {
    return override;
  }
  let url;
  try {
    url = new URL(listenUrl);
  } catch {
    return listenUrl;
  }
  if (
    url.hostname === "0.0.0.0" ||
    url.hostname === "::" ||
    url.hostname === "[::]"
  ) {
    const lanAddress = firstLanIpv4Address(
      typeof options.networkInterfaces === "function"
        ? options.networkInterfaces()
        : options.networkInterfaces,
    ) ?? "127.0.0.1";
    url.hostname = lanAddress;
  }
  return url.toString();
}

function firstLanIpv4Address(networkInterfaces = os.networkInterfaces()) {
  for (const entries of Object.values(networkInterfaces)) {
    for (const entry of entries ?? []) {
      if (
        entry.family === "IPv4" &&
        !entry.internal &&
        typeof entry.address === "string"
      ) {
        return entry.address;
      }
    }
  }
  return null;
}

function mobileConnectionInfoEqual(left, right) {
  if (left === right) {
    return true;
  }
  return JSON.stringify(left) === JSON.stringify(right);
}

function writeTokenFile(tokenFile, token) {
  fs.mkdirSync(path.dirname(tokenFile), { recursive: true });
  fs.writeFileSync(tokenFile, `${token}\n`, { mode: 0o600 });
  fs.chmodSync(tokenFile, 0o600);
}

function ensureMorpheusHomeDefaults(morpheusHome, options = {}) {
  const mkdirSync = options.mkdirSync ?? fs.mkdirSync;
  const existsSync = options.existsSync ?? fs.existsSync;
  const readFileSync = options.readFileSync ?? fs.readFileSync;
  const writeFileSync = options.writeFileSync ?? fs.writeFileSync;
  mkdirSync(morpheusHome, { recursive: true });

  const compactPromptPath = path.join(
    morpheusHome,
    HOME_COMPACT_PROMPT_RELATIVE_PATH,
  );
  mkdirSync(path.dirname(compactPromptPath), { recursive: true });
  if (existsSync(compactPromptPath)) {
    return {
      compactPromptPath,
      seededCompactPrompt: false,
      compactPromptSeedPath: null,
    };
  }

  const seedPath = findDefaultCompactPromptSeedPath(options);
  if (!seedPath) {
    options.warn?.(
      `Default compact prompt seed was not found; ${compactPromptPath} was not created.`,
    );
    return {
      compactPromptPath,
      seededCompactPrompt: false,
      compactPromptSeedPath: null,
    };
  }

  const content = readFileSync(seedPath, "utf8");
  writeFileSync(compactPromptPath, content);
  return {
    compactPromptPath,
    seededCompactPrompt: true,
    compactPromptSeedPath: seedPath,
  };
}

function findDefaultCompactPromptSeedPath(options = {}) {
  const existsSync = options.existsSync ?? fs.existsSync;
  const explicitSeedPath = options.defaultCompactPromptSeedPath;
  if (explicitSeedPath && existsSync(explicitSeedPath)) {
    return explicitSeedPath;
  }

  const resourcesPath = options.resourcesPath ?? currentResourcesPath();
  if (resourcesPath) {
    const packagedSeedPath = path.join(
      resourcesPath,
      PACKAGED_COMPACT_PROMPT_RELATIVE_PATH,
    );
    if (existsSync(packagedSeedPath)) {
      return packagedSeedPath;
    }
  }

  const sourceSeedPath = findWorkspacePath(
    path.join("codex-rs", "thread-service", "templates", "compact", "prompt.md"),
    options,
  );
  if (sourceSeedPath) {
    return sourceSeedPath;
  }

  return null;
}

function canBindWebSocketListenUrl(listenUrl) {
  const parsed = parseMobileWebSocketListenUrl(listenUrl);
  if (!parsed) {
    return Promise.resolve(false);
  }
  return new Promise((resolve) => {
    const server = net.createServer();
    let settled = false;
    const settle = (available) => {
      if (settled) {
        return;
      }
      settled = true;
      server.removeAllListeners();
      if (server.listening) {
        server.close(() => resolve(available));
      } else {
        resolve(available);
      }
    };
    server.once("error", () => settle(false));
    server.once("listening", () => settle(true));
    server.listen({
      host: parsed.host,
      port: parsed.port,
      exclusive: true,
    });
  });
}

function parseMobileWebSocketListenUrl(listenUrl) {
  if (!listenUrl.startsWith("ws://")) {
    return null;
  }
  const socketAddress = listenUrl.slice("ws://".length);
  if (
    !socketAddress ||
    socketAddress.includes("/") ||
    socketAddress.includes("?") ||
    socketAddress.includes("#")
  ) {
    return null;
  }

  let host;
  let portString;
  if (socketAddress.startsWith("[")) {
    const closingBracket = socketAddress.indexOf("]");
    if (closingBracket === -1 || socketAddress[closingBracket + 1] !== ":") {
      return null;
    }
    host = socketAddress.slice(1, closingBracket);
    portString = socketAddress.slice(closingBracket + 2);
  } else {
    const separator = socketAddress.lastIndexOf(":");
    if (separator === -1 || socketAddress.indexOf(":") !== separator) {
      return null;
    }
    host = socketAddress.slice(0, separator);
    portString = socketAddress.slice(separator + 1);
  }

  const port = Number(portString);
  if (
    net.isIP(host) === 0 ||
    !Number.isInteger(port) ||
    port < 1 ||
    port > 65535 ||
    String(port) !== portString
  ) {
    return null;
  }

  return { host, port };
}

function formatMobileWebSocketListenUrl(host, port) {
  const formattedHost = host.includes(":") ? `[${host}]` : host;
  return `ws://${formattedHost}:${port}`;
}

function resolveDefaultAppServerBinary(options = {}) {
  const packagedBinary = findPackagedAppServerBinaryPath(options);
  if (packagedBinary) {
    return shellQuote(packagedBinary);
  }
  return resolveWorkspaceAppServerBinary(options);
}

function findPackagedAppServerBinaryPath(options = {}) {
  const existsSync = options.existsSync ?? fs.existsSync;
  const resourcesPath = options.resourcesPath ?? currentResourcesPath();
  if (resourcesPath) {
    const packagedBinary = path.join(
      resourcesPath,
      PACKAGED_APP_SERVER_RELATIVE_PATH,
    );
    if (existsSync(packagedBinary)) {
      return packagedBinary;
    }
  }
  return null;
}

function resolveWorkspaceAppServerBinary(options = {}) {
  const workspaceBinary = findWorkspacePath(
    path.join("codex-rs", "target", "debug", "app-server"),
    options,
  );
  if (workspaceBinary) {
    return shellQuote(workspaceBinary);
  }
  const workspaceReleaseBinary = findWorkspacePath(
    path.join("codex-rs", "target", "release", "app-server"),
    options,
  );
  if (workspaceReleaseBinary) {
    return shellQuote(workspaceReleaseBinary);
  }
  return "app-server";
}

function findWorkspacePath(relativePath, options = {}) {
  const existsSync = options.existsSync ?? fs.existsSync;
  const startDir = options.startDir ?? __dirname;
  for (
    let current = startDir;
    current !== path.dirname(current);
    current = path.dirname(current)
  ) {
    const candidate = path.join(current, relativePath);
    if (existsSync(candidate)) {
      return candidate;
    }
  }
  return null;
}

function currentResourcesPath() {
  return typeof process.resourcesPath === "string"
    ? process.resourcesPath
    : null;
}

function resolvePrototypeMorpheusHome(baseEnv = process.env) {
  return path.join(baseEnv.HOME ?? os.homedir(), ".morpheus");
}

function shellQuote(value) {
  return `"${value.replaceAll('"', '\\"')}"`;
}

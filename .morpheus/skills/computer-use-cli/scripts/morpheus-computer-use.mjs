#!/usr/bin/env node

import { execFile, spawn } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { basename, join } from "node:path";
import { EOL, homedir } from "node:os";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);

const DEFAULT_NODE_REPL =
  "/Applications/ChatGPT.app/Contents/Resources/cua_node/bin/node_repl";
const DEFAULT_NODE_MODULES =
  "/Applications/ChatGPT.app/Contents/Resources/cua_node/lib/node_modules";
const SIDE_EFFECT_COMMANDS = new Set([
  "click",
  "move",
  "drag",
  "type",
  "paste",
  "press-key",
  "scroll",
  "set-value",
  "launch-app",
]);

class CliError extends Error {
  constructor(message, exitCode = 1) {
    super(message);
    this.exitCode = exitCode;
  }
}

class MacosNativeBackend {
  constructor(options) {
    this.options = options;
  }

  async listApps() {
    const [installed, running] = await Promise.all([
      listInstalledApps(),
      listRunningApps(),
    ]);
    const byId = new Map();
    const byName = new Map();
    const byBundle = new Map();
    const remember = (app) => {
      byId.set(app.id, app);
      if (app.path) {
        byId.set(app.path, app);
      }
      if (app.bundleIdentifier) {
        byBundle.set(app.bundleIdentifier, app);
      }
      byName.set(normalizeAppName(displayName(app)), app);
    };
    for (const app of installed) {
      remember(app);
    }
    for (const app of running) {
      const existing =
        (app.bundleIdentifier ? byBundle.get(app.bundleIdentifier) : null) ??
        byId.get(app.id) ??
        byName.get(normalizeAppName(displayName(app)));
      if (existing) {
        existing.isRunning = true;
        existing.processName = app.processName;
        existing.bundleIdentifier = existing.bundleIdentifier ?? app.bundleIdentifier;
      } else {
        remember(app);
      }
    }
    const uniqueApps = [...new Set(byId.values())];
    return {
      backend: "macos-native",
      apps: uniqueApps.sort((left, right) =>
        displayName(left).localeCompare(displayName(right)),
      ),
    };
  }

  async getAppState() {
    const app = this.options.app;
    if (!app) {
      throw new CliError("get-app-state requires --app", 2);
    }
    const state = await readAccessibilityTree(app);
    return {
      backend: "macos-native",
      app,
      ...state,
    };
  }
}

class BundledSkyBackend {
  constructor(options) {
    this.options = options;
  }

  async listApps() {
    const client = new McpNodeReplClient(this.options);
    try {
      await client.start();
      return {
        backend: "bundled-sky",
        apps: unwrapCuaResult(
          await client.evalJson(cuaCall("cua.listApps({ emit: false })")),
        ),
      };
    } finally {
      await client.stop();
    }
  }

  async getAppState() {
    const app = this.options.app;
    if (!app) {
      throw new CliError("get-app-state requires --app", 2);
    }
    const client = new McpNodeReplClient(this.options);
    try {
      await client.start();
      const body = this.options.includeScreenshot
        ? `target.getAXStateAndScreenshot({ disableDiffing: ${JSON.stringify(
            this.options.disableDiff,
          )}, emit: false })`
        : `target.getAXState({ disableDiffing: ${JSON.stringify(
            this.options.disableDiff,
          )}, emit: false })`;
      return {
        backend: "bundled-sky",
        app,
        state: unwrapCuaResult(
          await client.evalJson(
            cuaCall(`(async () => {
              const target = await cua.getApp(${JSON.stringify(app)});
              return ${body};
            })()`),
          ),
        ),
      };
    } finally {
      await client.stop();
    }
  }
}

class McpNodeReplClient {
  constructor({ nodeReplPath, moduleDir, allowReadApproval }) {
    this.nodeReplPath = nodeReplPath;
    this.moduleDir = moduleDir;
    this.allowReadApproval = allowReadApproval;
    this.nextId = 1;
    this.pending = new Map();
    this.buffer = "";
    this.child = null;
  }

  async start() {
    assertFile(this.nodeReplPath, "node_repl runtime");
    assertFile(this.moduleDir, "bundled node_modules directory");
    this.child = spawn(this.nodeReplPath, [], {
      stdio: ["pipe", "pipe", "pipe"],
      env: buildNodeReplEnv(this.moduleDir),
    });
    this.child.stdout.setEncoding("utf8");
    this.child.stderr.setEncoding("utf8");
    this.child.stdout.on("data", (chunk) => this.#onStdout(chunk));
    this.child.stderr.on("data", (chunk) => {
      if (process.env.MORPHEUS_CUA_DEBUG === "1") {
        process.stderr.write(chunk);
      }
    });
    this.child.on("exit", (code, signal) => {
      const err = new Error(
        `node_repl exited before completing request (code=${code}, signal=${signal})`,
      );
      for (const { reject } of this.pending.values()) {
        reject(err);
      }
      this.pending.clear();
    });

    await this.request("initialize", {
      protocolVersion: "2024-11-05",
      capabilities: { elicitation: {} },
      clientInfo: { name: "morpheus-computer-use", version: "0.1.0" },
    });
    this.notify("notifications/initialized", {});
    await this.callTool("js_add_node_module_dir", { path: this.moduleDir });
  }

  async stop() {
    if (!this.child) {
      return;
    }
    try {
      await this.callTool("turn_ended", {});
    } catch {
      // Best effort cleanup.
    }
    this.child.kill("SIGTERM");
    this.child = null;
  }

  request(method, params = {}) {
    if (!this.child) {
      return Promise.reject(new Error("node_repl is not running"));
    }
    const id = this.nextId++;
    const message = { jsonrpc: "2.0", id, method, params };
    return new Promise((resolvePromise, rejectPromise) => {
      this.pending.set(id, { resolve: resolvePromise, reject: rejectPromise });
      this.#send(message);
    });
  }

  notify(method, params = {}) {
    this.#send({ jsonrpc: "2.0", method, params });
  }

  async callTool(name, args) {
    const result = await this.request("tools/call", {
      name,
      arguments: args,
    });
    if (result?.isError) {
      throw new Error(extractToolText(result) || `${name} failed`);
    }
    return result;
  }

  async evalJson(code) {
    const result = await this.callTool("js", { code });
    if (process.env.MORPHEUS_CUA_DEBUG === "1") {
      process.stderr.write(`node_repl js raw result: ${JSON.stringify(result)}${EOL}`);
    }
    const text = extractToolText(result);
    if (!text) {
      return null;
    }
    try {
      return JSON.parse(extractLastJsonObject(text));
    } catch (error) {
      throw new Error(`failed to parse node_repl JSON result: ${error.message}\n${text}`);
    }
  }

  #send(message) {
    this.child.stdin.write(`${JSON.stringify(message)}\n`);
  }

  #onStdout(chunk) {
    this.buffer += chunk;
    for (;;) {
      const newline = this.buffer.indexOf("\n");
      if (newline === -1) {
        break;
      }
      const line = this.buffer.slice(0, newline).trim();
      this.buffer = this.buffer.slice(newline + 1);
      if (line) {
        this.#handleLine(line);
      }
    }
  }

  #handleLine(line) {
    let message;
    try {
      message = JSON.parse(line);
    } catch {
      if (process.env.MORPHEUS_CUA_DEBUG === "1") {
        process.stderr.write(`non-json node_repl stdout: ${line}${EOL}`);
      }
      return;
    }

    if (message.id !== undefined && (message.result !== undefined || message.error)) {
      const pending = this.pending.get(message.id);
      if (!pending) {
        return;
      }
      this.pending.delete(message.id);
      if (message.error) {
        pending.reject(new Error(message.error.message ?? JSON.stringify(message.error)));
      } else {
        pending.resolve(message.result);
      }
      return;
    }

    if (message.id !== undefined && message.method === "elicitation/create") {
      this.#handleElicitation(message);
    }
  }

  #handleElicitation(message) {
    if (this.allowReadApproval === "session") {
      this.#send({
        jsonrpc: "2.0",
        id: message.id,
        result: {
          action: "accept",
          content: {},
          _meta: { persist: "session" },
        },
      });
      return;
    }
    this.#send({
      jsonrpc: "2.0",
      id: message.id,
      result: {
        action: "cancel",
        content: {},
      },
    });
  }
}

function usage() {
  return `Usage:
  morpheus-computer-use list-apps [--json] [--preview-chars N] [--backend macos-native|bundled-sky]
  morpheus-computer-use get-app-state --app APP [--json] [--preview-chars N] [--backend macos-native|bundled-sky] [--allow-read-approval session] [--diff] [--no-screenshot]

Read-only commands default to Morpheus-owned macOS inspection. The bundled-sky
backend is an adapter for ChatGPT's trusted node_repl + Computer Use runtime.
Side-effect commands are intentionally disabled in this MVP; future
click/drag/type flows must use a persistent server/repl or one batch command,
not separate stateless CLI processes.

Options:
  --backend NAME                macos-native (default) or bundled-sky.
  --node-repl PATH              Override node_repl path for bundled-sky.
  --node-modules PATH           Override cua_node node_modules path for bundled-sky.
  --json                        Emit JSON envelope.
  --preview-chars N             Bound stdout preview (default: 20000).
  --allow-read-approval session Allow read-only app approval for bundled-sky.
  --app VALUE                   App name, bundle id, process id, or full .app path.
  --diff                        Allow backend diff output for get-app-state.
  --no-screenshot               Request state without screenshot when supported.
  --help                        Show help.`;
}

function parseArgs(argv) {
  const args = [...argv];
  if (args[0] === "--help" || args[0] === "-h") {
    return defaultOptions({ help: true });
  }
  const command = args.shift();
  const options = defaultOptions({ command });

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    switch (arg) {
      case "--help":
      case "-h":
        options.help = true;
        break;
      case "--json":
        options.json = true;
        break;
      case "--diff":
        options.disableDiff = false;
        break;
      case "--no-screenshot":
        options.includeScreenshot = false;
        break;
      case "--backend":
        options.backend = requiredValue(args, ++index, arg);
        if (!["macos-native", "bundled-sky"].includes(options.backend)) {
          throw new CliError("--backend must be macos-native or bundled-sky", 2);
        }
        break;
      case "--node-repl":
        options.nodeReplPath = requiredValue(args, ++index, arg);
        break;
      case "--node-modules":
        options.moduleDir = requiredValue(args, ++index, arg);
        break;
      case "--preview-chars":
        options.previewChars = parsePositiveInt(requiredValue(args, ++index, arg), arg);
        break;
      case "--app":
        options.app = requiredValue(args, ++index, arg);
        break;
      case "--allow-read-approval":
        options.allowReadApproval = requiredValue(args, ++index, arg);
        if (options.allowReadApproval !== "session") {
          throw new CliError("--allow-read-approval only supports 'session'", 2);
        }
        break;
      default:
        throw new CliError(`unknown option: ${arg}\n\n${usage()}`, 2);
    }
  }
  return options;
}

function defaultOptions(overrides = {}) {
  return {
    command: null,
    backend: process.env.MORPHEUS_CUA_BACKEND ?? "macos-native",
    nodeReplPath: process.env.MORPHEUS_CUA_NODE_REPL ?? DEFAULT_NODE_REPL,
    moduleDir: process.env.MORPHEUS_CUA_NODE_MODULES ?? DEFAULT_NODE_MODULES,
    json: false,
    previewChars: 20_000,
    app: null,
    allowReadApproval: null,
    disableDiff: true,
    includeScreenshot: true,
    help: false,
    ...overrides,
  };
}

function createBackend(options) {
  if (options.backend === "bundled-sky") {
    return new BundledSkyBackend(options);
  }
  return new MacosNativeBackend(options);
}

async function listInstalledApps() {
  try {
    const { stdout } = await execFileAsync("/usr/bin/mdfind", [
      "kMDItemContentType == 'com.apple.application-bundle'",
    ], { maxBuffer: 16 * 1024 * 1024 });
    return stdout
      .split(/\r?\n/)
      .map((path) => path.trim())
      .filter((path) => path.endsWith(".app"))
      .map((path) => ({
        id: path,
        path,
        displayName: basename(path, ".app"),
        isRunning: false,
      }));
  } catch {
    return [];
  }
}

async function listRunningApps() {
  const script = `set appRows to {}
tell application "System Events"
  repeat with proc in (every process whose background only is false)
    set procName to name of proc
    set bundleId to ""
    try
      set bundleId to bundle identifier of proc
    end try
    copy (procName & tab & bundleId) to end of appRows
  end repeat
end tell
set AppleScript's text item delimiters to linefeed
return appRows as text`;
  const { stdout } = await execFileAsync("/usr/bin/osascript", ["-e", script]);
  return stdout
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => {
      const [processName, bundleIdentifier = ""] = line.split("\t");
      return {
        id: bundleIdentifier || processName,
        processName,
        displayName: processName,
        bundleIdentifier: bundleIdentifier || undefined,
        isRunning: true,
      };
    });
}

async function readAccessibilityTree(app) {
  const script = `on env(name)
  return system attribute name
end env

on firstText(x)
  try
    return x as text
  on error
    return ""
  end try
end firstText

set nodecount to 0
set maxnodes to 2000
set charcount to 0
set maxchars to 200000
set axtruncated to false

on describeElement(elem, dep, maxd, maxc)
  global nodecount, maxnodes, charcount, maxchars, axtruncated
  if axtruncated then return ""
  if dep > maxd then return ""
  set nodecount to nodecount + 1
  set indent to ""
  if dep > 0 then
    repeat with n from 1 to dep
      set indent to indent & "  "
    end repeat
  end if
  set roletext to ""
  set nametext to ""
  set valuetext to ""
  tell application "System Events"
    try
      set roletext to role of elem as text
    end try
    try
      set nametext to name of elem as text
    end try
    try
      set valuetext to value of elem as text
    end try
  end tell
  set linetext to indent & roletext
  if nametext is not "" then set linetext to linetext & " name=" & quoted form of nametext
  if valuetext is not "" then set linetext to linetext & " value=" & quoted form of valuetext
  set outtext to linetext & linefeed
  set charcount to charcount + (length of outtext)
  if nodecount > maxnodes or charcount > maxchars then
    set axtruncated to true
    return indent & "... truncated by native budget" & linefeed
  end if
  try
    tell application "System Events"
      set kids to UI elements of elem
      set kidcount to count of kids
    end tell
    set limitcount to kidcount
    if limitcount > maxc then set limitcount to maxc
    repeat with idx from 1 to limitcount
      set outtext to outtext & (my describeElement(item idx of kids, dep + 1, maxd, maxc))
    end repeat
    if kidcount > maxc then set outtext to outtext & indent & "  ... truncated " & (kidcount - maxc) & " children" & linefeed
  end try
  return outtext
end describeElement

set targetapp to env("MORPHEUS_CUA_APP")
set matched to missing value
tell application "System Events"
  repeat with proc in every process
    try
      set procname to name of proc as text
      set bundleid to ""
      try
        set bundleid to bundle identifier of proc as text
      end try
      if procname is targetapp or bundleid is targetapp or targetapp ends with (procname & ".app") then
        set matched to proc
        exit repeat
      end if
    end try
  end repeat
  if matched is missing value then error "running app not found: " & targetapp
  set treetext to my describeElement(matched, 0, 5, 120)
  set windownames to {}
  try
    repeat with win in windows of matched
      copy (my firstText(name of win)) to end of windownames
    end repeat
  end try
end tell
set AppleScript's text item delimiters to linefeed
return "__MORPHEUS_META__" & (axtruncated as text) & tab & nodecount & tab & charcount & linefeed & (windownames as text) & linefeed & "--- AX TREE ---" & linefeed & treetext`;
  try {
    const { stdout } = await execFileAsync("/usr/bin/osascript", ["-e", script], {
      env: { ...process.env, MORPHEUS_CUA_APP: app },
      maxBuffer: 8 * 1024 * 1024,
    });
    const lines = stdout.split(/\r?\n/);
    const metaLine = lines[0]?.startsWith("__MORPHEUS_META__")
      ? lines.shift()
      : null;
    const [truncatedText, nodeCountText, charCountText] = (metaLine ?? "")
      .replace("__MORPHEUS_META__", "")
      .split("\t");
    const [windowsText, treeText = ""] = lines.join("\n").split("\n--- AX TREE ---\n");
    return {
      windows: windowsText.split(/\r?\n/).map((line) => line.trim()).filter(Boolean),
      accessibilityText: treeText.trim(),
      truncated: truncatedText === "true",
      nodeCount: Number(nodeCountText) || null,
      charCount: Number(charCountText) || null,
      screenshot: null,
    };
  } catch (error) {
    throw new Error(
      `macos-native get-app-state failed. Ensure the target app is running and Accessibility/Automation permissions allow osascript. ${error.message}`,
    );
  }
}

function requiredValue(args, index, flag) {
  const value = args[index];
  if (!value || value.startsWith("--")) {
    throw new CliError(`${flag} requires a value`, 2);
  }
  return value;
}

function parsePositiveInt(value, flag) {
  const parsed = Number(value);
  if (!Number.isInteger(parsed) || parsed < 0) {
    throw new CliError(`${flag} requires a non-negative integer`, 2);
  }
  return parsed;
}

function assertFile(path, label) {
  if (!existsSync(path)) {
    throw new CliError(`${label} not found: ${path}`);
  }
}

function buildNodeReplEnv(moduleDir) {
  const defaultCodexHome = join(homedir(), ".codex");
  const defaults = {
    NODE_NO_WARNINGS: "1",
    NODE_REPL_NODE_MODULE_DIRS: moduleDir,
    NODE_REPL_NODE_PATH:
      "/Applications/ChatGPT.app/Contents/Resources/cua_node/bin/node",
    NODE_REPL_TRUSTED_CODE_PATHS: `${defaultCodexHome}:${moduleDir}`,
    NODE_REPL_TRUSTED_SERVICES: JSON.stringify({ sky: "@oai/sky/service" }),
    CUA_REPL_ENABLED_SURFACES: "computer",
    NODE_REPL_UNTRUSTED_ENV_ALLOWLIST: "CUA_REPL_ENABLED_SURFACES",
    SKY_CUA_SERVICE_PATH: join(
      defaultCodexHome,
      "computer-use",
      "Codex Computer Use.app",
    ),
    CODEX_HOME: defaultCodexHome,
  };
  const configured = readNodeReplConfigEnv(
    process.env.MORPHEUS_CUA_CONFIG_TOML ?? join(defaultCodexHome, "config.toml"),
  );
  const allowlist = [
    process.env.NODE_REPL_UNTRUSTED_ENV_ALLOWLIST,
    configured.NODE_REPL_UNTRUSTED_ENV_ALLOWLIST,
    "CUA_REPL_ENABLED_SURFACES",
  ]
    .filter(Boolean)
    .join(",");
  return {
    ...defaults,
    ...configured,
    ...process.env,
    NODE_NO_WARNINGS: process.env.NODE_NO_WARNINGS ?? configured.NODE_NO_WARNINGS ?? "1",
    NODE_REPL_NODE_MODULE_DIRS:
      process.env.NODE_REPL_NODE_MODULE_DIRS ??
      configured.NODE_REPL_NODE_MODULE_DIRS ??
      moduleDir,
    NODE_REPL_UNTRUSTED_ENV_ALLOWLIST: allowlist,
  };
}

function readNodeReplConfigEnv(path) {
  if (!existsSync(path)) {
    return {};
  }
  const contents = readFileSync(path, "utf8");
  const env = {};
  let inSection = false;
  for (const rawLine of contents.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line || line.startsWith("#")) {
      continue;
    }
    if (line.startsWith("[") && line.endsWith("]")) {
      inSection = line === "[mcp_servers.node_repl.env]";
      continue;
    }
    if (!inSection) {
      continue;
    }
    const match = line.match(/^([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.+)$/);
    if (match) {
      env[match[1]] = parseTomlString(match[2].trim());
    }
  }
  return env;
}

function parseTomlString(value) {
  if (value.startsWith('"') && value.endsWith('"')) {
    try {
      return JSON.parse(value);
    } catch {
      return value.slice(1, -1);
    }
  }
  if (value.startsWith("'") && value.endsWith("'")) {
    return value.slice(1, -1);
  }
  return value;
}

function extractToolText(result) {
  const parts = Array.isArray(result?.content) ? result.content : [];
  return parts
    .map((part) => (part?.type === "text" ? part.text ?? "" : part?.text ?? ""))
    .filter(Boolean)
    .join("\n")
    .trim();
}

function extractLastJsonObject(text) {
  const trimmed = text.trim();
  const lines = trimmed.split(/\r?\n/).map((line) => line.trim()).filter(Boolean);
  for (let index = lines.length - 1; index >= 0; index -= 1) {
    if (lines[index].startsWith("{") && lines[index].endsWith("}")) {
      return lines[index];
    }
  }
  return trimmed;
}

function bounded(value, maxChars) {
  const text = typeof value === "string" ? value : JSON.stringify(value, null, 2);
  if (maxChars === 0 || text.length <= maxChars) {
    return { text, truncated: false, omittedChars: 0 };
  }
  return {
    text: text.slice(0, maxChars),
    truncated: true,
    omittedChars: text.length - maxChars,
  };
}

function printResult(payload, options) {
  const preview = bounded(payload.data, options.previewChars);
  if (options.json) {
    console.log(
      JSON.stringify(
        {
          ok: true,
          command: options.command,
          backend: options.backend,
          truncated: preview.truncated,
          omittedChars: preview.omittedChars,
          data: preview.truncated ? preview.text : payload.data,
        },
        null,
        2,
      ),
    );
    return;
  }
  console.log(preview.text);
  if (preview.truncated) {
    console.error(`[truncated ${preview.omittedChars} chars; use --preview-chars 0 for full output]`);
  }
}

function displayName(app) {
  return app.displayName ?? app.processName ?? app.path ?? app.id;
}

function normalizeAppName(name) {
  return name.toLowerCase().replace(/\.app$/, "");
}

function cuaCall(source) {
  return `nodeRepl.write(JSON.stringify(await (async () => {
function serializeError(error) {
  if (!error || typeof error !== 'object') {
    return { message: String(error) };
  }
  return {
    name: error.name,
    message: error.message,
    stack: error.stack,
    cause: error.cause ? serializeError(error.cause) : undefined,
  };
}
try {
  await import('@oai/cua/tinyskyAlt');
  const data = await (${source});
  return { ok: true, data };
} catch (error) {
  return { ok: false, error: serializeError(error) };
}
})()))`;
}

function unwrapCuaResult(result) {
  if (result?.ok) {
    return result.data;
  }
  const error = result?.error;
  const details = process.env.MORPHEUS_CUA_DEBUG === "1" ? error?.stack : error?.message;
  throw new Error(details || "Computer Use request failed");
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  if (!options.command || options.help) {
    console.log(usage());
    return;
  }
  if (SIDE_EFFECT_COMMANDS.has(options.command)) {
    throw new CliError(
      `${options.command} is intentionally disabled. Use a future server/repl/batch session with explicit confirmation for side-effect GUI actions.`,
      2,
    );
  }
  if (!["list-apps", "get-app-state"].includes(options.command)) {
    throw new CliError(`unknown command: ${options.command}\n\n${usage()}`, 2);
  }

  const backend = createBackend(options);
  const data =
    options.command === "list-apps"
      ? await backend.listApps()
      : await backend.getAppState();
  printResult({ data }, options);
}

main().catch((error) => {
  const exitCode = error instanceof CliError ? error.exitCode : 1;
  const options = bestEffortOptions(process.argv.slice(2));
  const preview = bounded(error.message, options.previewChars);
  const envelope = {
    ok: false,
    command: options.command,
    backend: options.backend,
    truncated: preview.truncated,
    omittedChars: preview.omittedChars,
    error: preview.text,
    data: null,
  };
  if (process.argv.includes("--json")) {
    console.error(JSON.stringify(envelope, null, 2));
  } else {
    console.error(error.message);
  }
  process.exit(exitCode);
});

function bestEffortOptions(argv) {
  try {
    return parseArgs(argv);
  } catch {
    const command = argv.find((arg) => arg && !arg.startsWith("--")) ?? null;
    const backendIndex = argv.indexOf("--backend");
    const previewIndex = argv.indexOf("--preview-chars");
    return defaultOptions({
      command,
      backend:
        backendIndex >= 0 && argv[backendIndex + 1]
          ? argv[backendIndex + 1]
          : process.env.MORPHEUS_CUA_BACKEND ?? "macos-native",
      previewChars: safePreviewChars(argv[previewIndex + 1]),
    });
  }
}

function safePreviewChars(value) {
  const parsed = Number(value);
  return Number.isInteger(parsed) && parsed >= 0 ? parsed : 20_000;
}

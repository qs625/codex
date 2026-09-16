#!/usr/bin/env node

import { createRequire } from "node:module";
import { spawn } from "node:child_process";
import { dirname, join } from "node:path";
import { randomUUID } from "node:crypto";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const {
  createComputerUseManager,
} = require("../apps/root-worker-prototype/electron/computerUse.cjs");

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const CLI_DISABLED_ACTIONS = new Set(["click", "type", "key", "drag"]);
const DEFAULT_ACTIONS = [{ type: "start" }];
const OVERLAY_HELPER_PATH = join(
  __dirname,
  "morpheus-computer-use-overlay-helper.cjs",
);

export async function runComputerUseCli({
  argv = process.argv.slice(2),
  managerFactory = null,
  overlayControllerFactory = createCliOverlayController,
  writeStdout = null,
  writeStderr = null,
} = {}) {
  try {
    const request = parseComputerUseCliArgs(argv);
    if (request.help) {
      const output = usage();
      writeStdout?.(`${output}\n`);
      return { ok: true, help: output, exitCode: 0 };
    }
    const result = await runComputerUseRequest(
      request,
      managerFactory ??
        createComputerUseCliManagerFactory({ overlayControllerFactory }),
    );
    writeStdout?.(`${JSON.stringify(result, null, request.pretty ? 2 : 0)}\n`);
    return { ...result, exitCode: result.ok ? 0 : 1 };
  } catch (error) {
    const result = {
      ok: false,
      error: errorMessage(error),
      usage: usage(),
    };
    writeStderr?.(`${JSON.stringify(result, null, 2)}\n`);
    return { ...result, exitCode: 1 };
  }
}

export async function runComputerUseRequest(request, managerFactory) {
  const manager = await managerFactory(request);
  let started = false;
  let needsCleanup = false;
  const results = [];
  try {
    for (const [index, rawAction] of request.actions.entries()) {
      const action = normalizeCliAction(rawAction);
      if (action.type === "start") {
        const state = await manager.startSession({
          app: action.app ?? request.app ?? undefined,
        });
        started = true;
        needsCleanup = true;
        results.push(completedResult(index, action, state, request));
        continue;
      }
      if (action.type === "observe") {
        if (!started) {
          const state = await manager.startSession({ app: request.app ?? undefined });
          started = true;
          needsCleanup = true;
          results.push(completedResult(index, action, state, request));
          continue;
        }
        const state = await manager.observe("cli");
        results.push(completedResult(index, action, state, request));
        continue;
      }
      if (action.type === "stop") {
        const state = await manager.stopSession();
        started = false;
        needsCleanup = false;
        results.push(completedResult(index, action, state, request));
        continue;
      }
      if (CLI_DISABLED_ACTIONS.has(action.type)) {
        results.push(blockedResult(index, action, manager.state(), request));
        continue;
      }
      if (action.type === "move") {
        if (!started) {
          await manager.startSession({ app: request.app ?? undefined });
          started = true;
          needsCleanup = true;
        }
        const state = await manager.act(action);
        results.push(completedResult(index, action, state, request));
        if (state.overlay?.visible === true) {
          await delay(request.overlayHoldMs);
        }
        continue;
      }
      throw new Error(`Unsupported Computer Use CLI action: ${action.type}`);
    }
    needsCleanup = await cleanupBatchSession(manager, needsCleanup);
    const finalState = sanitizeState(manager.state(), request);
    return {
      ok: true,
      command: request.command,
      target: { app: request.app ?? null },
      policy: cliPolicy(),
      limitations: cliLimitations(),
      results,
      state: finalState,
    };
  } catch (error) {
    needsCleanup = await cleanupBatchSession(manager, needsCleanup);
    return {
      ok: false,
      command: request.command,
      target: { app: request.app ?? null },
      policy: cliPolicy(),
      limitations: cliLimitations(),
      results,
      state: safeManagerState(manager, request),
      error: errorMessage(error),
    };
  } finally {
    if (needsCleanup) {
      await manager.cleanup?.();
    }
  }
}

async function cleanupBatchSession(manager, needsCleanup) {
  if (!needsCleanup) {
    return false;
  }
  await manager.cleanup?.();
  return false;
}

export function parseComputerUseCliArgs(argv) {
  const args = [...argv];
  const command = args.shift() ?? "help";
  if (command === "help" || command === "--help" || command === "-h") {
    return { help: true };
  }
  if (command === "move") {
    throw new Error("Use `run --actions '[{\"type\":\"move\",\"x\":420,\"y\":360}]'` for move actions.");
  }

  const request = {
    command,
    app: null,
    actions: null,
    includeScreenshotData: true,
    pretty: false,
    noOverlay: false,
    overlayHoldMs: 900,
  };

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    switch (arg) {
      case "--app":
      case "--target-app":
        request.app = requireValue(args, ++index, arg);
        break;
      case "--actions":
        request.actions = JSON.parse(requireValue(args, ++index, arg));
        break;
      case "--json":
        break;
      case "--pretty":
        request.pretty = true;
        break;
      case "--include-screenshot-data":
        request.includeScreenshotData = true;
        break;
      case "--omit-screenshot-data":
        request.includeScreenshotData = false;
        break;
      case "--no-overlay":
        request.noOverlay = true;
        break;
      case "--overlay-hold-ms":
        request.overlayHoldMs = parseNonNegativeInteger(
          requireValue(args, ++index, arg),
          arg,
        );
        break;
      default:
        throw new Error(`Unsupported argument: ${arg}`);
    }
  }

  if (command === "run") {
    request.actions = normalizeCliActions(request.actions ?? DEFAULT_ACTIONS);
    return request;
  }
  if (["start", "observe", "stop"].includes(command)) {
    request.command = "run";
    request.actions = [{ type: command }];
    return request;
  }
  throw new Error(`Unsupported command: ${command}`);
}

export function createComputerUseCliManagerFactory({
  nativeClient = null,
  overlayControllerFactory = createCliOverlayController,
} = {}) {
  return async (request) => {
    const overlayController = request.noOverlay
      ? null
      : await overlayControllerFactory(request);
    return createComputerUseManager({
      ...(nativeClient ? { nativeClient } : {}),
      overlayController,
    });
  };
}

export function createCliOverlayController(options = {}) {
  let electronPath;
  try {
    electronPath = options.electronPath ?? require("electron");
  } catch (error) {
    return createUnavailableOverlayController(
      `Electron overlay helper is unavailable: ${errorMessage(error)}`,
    );
  }
  if (typeof electronPath !== "string" || electronPath.length === 0) {
    return createUnavailableOverlayController(
      "Electron overlay helper is unavailable: electron executable was not resolved.",
    );
  }
  return new CliOverlayController({
    electronPath,
    helperPath: options.helperPath ?? OVERLAY_HELPER_PATH,
    spawnProcess: options.spawnProcess ?? spawn,
  });
}

class CliOverlayController {
  constructor({ electronPath, helperPath, spawnProcess }) {
    this.electronPath = electronPath;
    this.helperPath = helperPath;
    this.spawnProcess = spawnProcess;
    this.child = null;
    this.pending = new Map();
    this.buffer = "";
    this.stderr = "";
    this.closed = false;
  }

  async update(payload) {
    return this.request({ type: "update", payload });
  }

  async destroy() {
    if (!this.child || this.closed) {
      return { available: true, visible: false, destroyed: true };
    }
    try {
      await this.request({ type: "destroy" });
      await this.request({ type: "quit" });
    } finally {
      this.child?.kill?.();
      this.child = null;
      this.closed = true;
    }
    return { available: true, visible: false, destroyed: true };
  }

  async request(message) {
    const child = this.ensureChild();
    const id = randomUUID();
    const payload = { id, ...message };
    return await new Promise((resolve) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        resolve(
          createUnavailableOverlayResult(
            `Computer Use overlay helper timed out while handling ${message.type}.`,
          ),
        );
      }, 3000);
      this.pending.set(id, {
        resolve: (result) => {
          clearTimeout(timer);
          resolve(result);
        },
      });
      child.stdin.write(`${JSON.stringify(payload)}\n`, (error) => {
        if (!error) {
          return;
        }
        const pending = this.pending.get(id);
        this.pending.delete(id);
        clearTimeout(timer);
        pending?.resolve(
          createUnavailableOverlayResult(
            `Computer Use overlay helper write failed: ${errorMessage(error)}`,
          ),
        );
      });
    });
  }

  ensureChild() {
    if (this.child && !this.closed) {
      return this.child;
    }
    const child = this.spawnProcess(this.electronPath, [this.helperPath], {
      stdio: ["pipe", "pipe", "pipe"],
    });
    this.child = child;
    this.closed = false;
    child.stdout.setEncoding("utf8");
    child.stdout.on("data", (chunk) => this.handleStdout(chunk));
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk) => {
      this.stderr = `${this.stderr}${chunk}`.slice(-4000);
    });
    child.on("error", (error) => {
      this.rejectAll(
        `Computer Use overlay helper failed to start: ${errorMessage(error)}`,
      );
    });
    child.on("exit", (code, signal) => {
      this.closed = true;
      this.rejectAll(
        `Computer Use overlay helper exited before completing the request (code ${code ?? "null"}, signal ${signal ?? "null"}).`,
      );
    });
    return child;
  }

  handleStdout(chunk) {
    this.buffer += chunk;
    while (this.buffer.includes("\n")) {
      const index = this.buffer.indexOf("\n");
      const line = this.buffer.slice(0, index);
      this.buffer = this.buffer.slice(index + 1);
      if (!line.trim()) {
        continue;
      }
      let message;
      try {
        message = JSON.parse(line);
      } catch {
        continue;
      }
      const pending = this.pending.get(message.id);
      if (!pending) {
        continue;
      }
      this.pending.delete(message.id);
      if (message.ok) {
        pending.resolve(message);
      } else {
        pending.resolve(
          createUnavailableOverlayResult(
            message.error || "Computer Use overlay helper returned an error.",
          ),
        );
      }
    }
  }

  rejectAll(reason) {
    const suffix = this.stderr ? ` stderr: ${this.stderr}` : "";
    for (const [id, pending] of this.pending) {
      this.pending.delete(id);
      pending.resolve(createUnavailableOverlayResult(`${reason}${suffix}`));
    }
  }
}

function createUnavailableOverlayController(reason) {
  return {
    async update() {
      return createUnavailableOverlayResult(reason);
    },
    async destroy() {
      return createUnavailableOverlayResult(reason);
    },
  };
}

function createUnavailableOverlayResult(reason) {
  return { available: false, visible: false, reason };
}

function normalizeCliActions(actions) {
  if (!Array.isArray(actions)) {
    throw new Error("--actions must be a JSON array");
  }
  if (actions.length === 0) {
    return DEFAULT_ACTIONS;
  }
  return actions;
}

function normalizeCliAction(action) {
  if (!action || typeof action !== "object") {
    throw new Error("Computer Use CLI action must be an object");
  }
  if (typeof action.type !== "string" || action.type.length === 0) {
    throw new Error("Computer Use CLI action requires a type");
  }
  return action;
}

function completedResult(index, action, state, request) {
  return {
    index,
    action,
    status: "completed",
    state: sanitizeState(state, request),
  };
}

function blockedResult(index, action, state, request) {
  return {
    index,
    action,
    status: "blocked",
    policy: {
      kind: "disabled",
      allowed: false,
      reason:
        "This Computer Use CLI v1 only permits start, observe, move, and stop. Click, type, key, and drag require a future confirmation boundary.",
    },
    state: sanitizeState(state, request),
  };
}

function safeManagerState(manager, request) {
  try {
    return sanitizeState(manager.state(), request);
  } catch {
    return null;
  }
}

function sanitizeState(state, request) {
  const copy = JSON.parse(JSON.stringify(state ?? null));
  if (!copy) {
    return copy;
  }
  stripScreenshotData(copy.observation?.screenshot, request);
  for (const item of copy.trace ?? []) {
    stripScreenshotData(item.screenshot, request);
  }
  return copy;
}

function stripScreenshotData(screenshot, request) {
  if (!screenshot) {
    return;
  }
  if (Object.hasOwn(screenshot, "path")) {
    delete screenshot.path;
    screenshot.pathOmitted =
      "Temporary screenshot file is cleaned up after the CLI run; use dataUrl or --omit-screenshot-data for metadata-only output.";
  }
  if (!request.includeScreenshotData && Object.hasOwn(screenshot, "dataUrl")) {
    delete screenshot.dataUrl;
    screenshot.dataUrlOmitted = true;
  }
}

function cliPolicy() {
  return {
    version: "computer-use-cli-v1",
    sessionMode: "single-process-batch",
    allowedActions: ["start", "observe", "move", "stop"],
    disabledActions: [...CLI_DISABLED_ACTIONS],
    moveDoesNotMoveSystemCursor: true,
  };
}

function cliLimitations() {
  return [
    "The CLI keeps session state only for the lifetime of one `run` process.",
    "The CLI v1 disables click, type, key, and drag even when the underlying manager can classify some of them as low risk.",
    "Screenshots include a bounded data URL by default; pass --omit-screenshot-data for metadata-only output.",
    "Visible agent cursor feedback is target-bound. Background targets are not activated or drawn over unrelated foreground apps.",
    "After a visible move, the CLI waits --overlay-hold-ms before the next action so the cursor can be seen.",
  ];
}

function requireValue(args, index, flag) {
  const value = args[index];
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`${flag} requires a value`);
  }
  return value;
}

function parseNonNegativeInteger(value, flag) {
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < 0) {
    throw new Error(`${flag} requires a non-negative integer`);
  }
  return parsed;
}

async function delay(ms) {
  if (!Number.isSafeInteger(ms) || ms <= 0) {
    return;
  }
  await new Promise((resolve) => setTimeout(resolve, ms));
}

function usage() {
  return [
    "Usage:",
    "  node scripts/morpheus-computer-use.mjs run --app <bundle-or-name> --json --actions '<json-array>'",
    "",
    "Actions:",
    "  start, observe, move, stop",
    "",
    "Example:",
    "  node scripts/morpheus-computer-use.mjs run --app com.apple.finder --json --actions '[{\"type\":\"start\"},{\"type\":\"observe\"},{\"type\":\"move\",\"x\":420,\"y\":360},{\"type\":\"stop\"}]'",
    "",
    "Options:",
    "  --omit-screenshot-data  Return screenshot metadata without the bounded data URL.",
    "  --overlay-hold-ms <ms>  Keep a visible moved cursor on screen before the next action (default: 900).",
  ].join("\n");
}

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const result = await runComputerUseCli({
    writeStdout: (value) => process.stdout.write(value),
    writeStderr: (value) => process.stderr.write(value),
  });
  process.exitCode = result.exitCode;
}

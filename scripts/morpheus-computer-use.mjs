#!/usr/bin/env node

import { createRequire } from "node:module";
import { spawn } from "node:child_process";
import { dirname, join } from "node:path";
import { randomUUID } from "node:crypto";
import { fileURLToPath } from "node:url";
import { createInterface } from "node:readline";

const require = createRequire(import.meta.url);
const {
  createComputerUseManager,
} = require("../apps/root-worker-prototype/electron/computerUse.cjs");

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const CLI_ACTIONS = new Set([
  "move",
  "click",
  "doubleClick",
  "rightClick",
  "scroll",
  "findText",
  "clickText",
  "setText",
  "type",
  "key",
  "hotkey",
  "drag",
  "wait",
  "pause",
]);
const DEFAULT_ACTIONS = [{ type: "start" }];
const OVERLAY_HELPER_PATH = join(
  __dirname,
  "morpheus-computer-use-overlay-helper.cjs",
);

export async function runComputerUseCli({
  argv = process.argv.slice(2),
  input = process.stdin,
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
    if (request.command === "repl") {
      const result = await runComputerUseRepl(
        request,
        managerFactory ??
          createComputerUseCliManagerFactory({ overlayControllerFactory }),
        {
          input,
          writeStdout,
        },
      );
      return { ...result, exitCode: result.ok ? 0 : 1 };
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
  const context = createSessionContext();
  const results = [];
  try {
    for (const [index, rawAction] of request.actions.entries()) {
      const { result } = await executeComputerUseSessionAction({
        manager,
        rawAction,
        request,
        context,
        index,
      });
      results.push(result);
      if (result.status === "blocked" || result.status === "failed") {
        context.needsCleanup = await cleanupBatchSession(manager, context.needsCleanup);
        break;
      }
    }
    context.needsCleanup = await cleanupBatchSession(manager, context.needsCleanup);
    const finalState = sanitizeState(manager.state(), request);
    return {
      ok: true,
      command: request.command,
      target: { app: request.app ?? null },
      actions: request.actions,
      policy: cliPolicy(request),
      limitations: cliLimitations(),
      results,
      state: finalState,
    };
  } catch (error) {
    context.needsCleanup = await cleanupBatchSession(manager, context.needsCleanup);
    return {
      ok: false,
      command: request.command,
      target: { app: request.app ?? null },
      actions: request.actions,
      policy: cliPolicy(request),
      limitations: cliLimitations(),
      results,
      state: safeManagerState(manager, request),
      error: errorMessage(error),
    };
  } finally {
    if (context.needsCleanup) {
      await manager.cleanup?.();
    }
  }
}

export async function runComputerUseRepl(
  request,
  managerFactory,
  { input = process.stdin, writeStdout = null, interruptSignal = null } = {},
) {
  const manager = await managerFactory(request);
  const context = createSessionContext();
  const sessionId = randomUUID();
  let ok = true;
  let index = 0;
  let cleanup = null;
  const settings = {
    json: request.json === true,
    raw: request.raw === true,
    traceTail: request.traceTail,
  };

  const write = (value) => writeStdout?.(`${value}\n`);
  const lines = createInterface({ input, crlfDelay: Infinity });
  let interrupted = false;
  const interrupt = () => {
    interrupted = true;
    lines.close();
  };
  const removeInterruptHandlers = installReplInterruptHandlers({
    lines,
    interrupt,
    interruptSignal,
  });
  try {
    for await (const line of lines) {
      if (interrupted) {
        break;
      }
      const parsed = parseReplLine(line);
      if (parsed.kind === "empty") {
        continue;
      }
      if (parsed.kind === "exit") {
        cleanup = await cleanupReplSession(manager, context, request);
        write(formatReplControlResult({ sessionId, command: "exit", cleanup }, settings));
        break;
      }
      if (parsed.kind === "help") {
        write(formatReplControlResult({ sessionId, command: "help", help: replHelp() }, settings));
        continue;
      }
      if (parsed.kind === "status") {
        write(
          formatReplStateResult({
            sessionId,
            command: "status",
            state: safeManagerState(manager, request),
          }, settings),
        );
        continue;
      }
      if (parsed.kind === "trace") {
        write(
          formatReplTraceResult({
            sessionId,
            command: "trace",
            state: safeManagerState(manager, request),
            count: parsed.count ?? settings.traceTail,
          }, settings),
        );
        continue;
      }
      if (parsed.kind === "mode") {
        settings[parsed.mode] = parsed.value;
        write(formatReplControlResult({
          sessionId,
          command: parsed.mode,
          [parsed.mode]: parsed.value,
        }, settings));
        continue;
      }

      const { action, result, traceItem } = await executeComputerUseSessionAction({
        manager,
        rawAction: parsed.action,
        request,
        context,
        index,
      });
      write(formatReplActionResult({ sessionId, action, result, traceItem }, settings));
      index += 1;
      if (interrupted) {
        break;
      }
      if (result.status === "blocked" || result.status === "failed") {
        ok = false;
        cleanup = await cleanupReplSession(manager, context, request);
        write(formatReplControlResult({
          sessionId,
          command: "cleanup",
          reason: result.status === "blocked" ? "policy-block" : "failed-action",
          cleanup,
        }, settings));
        break;
      }
    }
    if (interrupted) {
      ok = false;
      cleanup = await cleanupReplSession(manager, context, request);
      write(formatReplControlResult({
        sessionId,
        command: "cleanup",
        reason: "interrupt",
        cleanup,
      }, settings));
    } else if (!cleanup && context.needsCleanup) {
      cleanup = await cleanupReplSession(manager, context, request);
      write(formatReplControlResult({
        sessionId,
        command: "cleanup",
        reason: "eof",
        cleanup,
      }, settings));
    }
    return {
      ok,
      command: "repl",
      sessionId,
      cleanup,
      state: safeManagerState(manager, request),
    };
  } catch (error) {
    ok = false;
    cleanup = await cleanupReplSession(manager, context, request);
    write(formatReplControlResult({
      sessionId,
      command: "cleanup",
      reason: "error",
      cleanup,
      error: errorMessage(error),
    }, settings));
    return {
      ok: false,
      command: "repl",
      sessionId,
      cleanup,
      state: safeManagerState(manager, request),
      error: errorMessage(error),
    };
  } finally {
    removeInterruptHandlers();
    lines.close();
  }
}

function installReplInterruptHandlers({ lines, interrupt, interruptSignal }) {
  lines.once("SIGINT", interrupt);
  process.once("SIGINT", interrupt);
  if (interruptSignal) {
    if (interruptSignal.aborted) {
      interrupt();
    } else {
      interruptSignal.addEventListener("abort", interrupt, { once: true });
    }
  }
  return () => {
    lines.off("SIGINT", interrupt);
    process.off("SIGINT", interrupt);
    interruptSignal?.removeEventListener?.("abort", interrupt);
  };
}

function createSessionContext() {
  return {
    started: false,
    needsCleanup: false,
  };
}

async function executeComputerUseSessionAction({
  manager,
  rawAction,
  request,
  context,
  index,
}) {
  const action = normalizeCliAction(rawAction);
  if (action.type === "start") {
    const state = await manager.startSession({
      app: action.app ?? request.app ?? undefined,
    });
    context.started = true;
    context.needsCleanup = true;
    return { action, result: completedResult(index, action, state, request), traceItem: null };
  }
  if (action.type === "observe") {
    if (!context.started) {
      const state = await manager.startSession({ app: request.app ?? undefined });
      context.started = true;
      context.needsCleanup = true;
      return { action, result: completedResult(index, action, state, request), traceItem: null };
    }
    const state = await manager.observe("cli");
    return { action, result: completedResult(index, action, state, request), traceItem: null };
  }
  if (action.type === "stop") {
    const state = await manager.stopSession();
    context.started = false;
    context.needsCleanup = false;
    return { action, result: completedResult(index, action, state, request), traceItem: null };
  }
  if (CLI_ACTIONS.has(action.type)) {
    if (!context.started) {
      await manager.startSession({ app: request.app ?? undefined });
      context.started = true;
      context.needsCleanup = true;
    }
    const traceCursor = managerTraceCursor(manager);
    const state = await manager.act(action);
    const traceItem = newTraceItemForAction(state, action, traceCursor);
    const result = managerActionResult(
      index,
      action,
      state,
      request,
      traceItem,
    );
    if (state.overlay?.visible === true) {
      await delay(request.overlayHoldMs);
    }
    return { action, result, traceItem: sanitizeTraceItem(traceItem, request) };
  }
  throw new Error(`Unsupported Computer Use CLI action: ${action.type}`);
}

async function cleanupBatchSession(manager, needsCleanup) {
  if (!needsCleanup) {
    return false;
  }
  await manager.cleanup?.();
  return false;
}

async function cleanupReplSession(manager, context, request) {
  if (!context.needsCleanup) {
    return { needed: false, status: "skipped" };
  }
  try {
    const stoppedState =
      typeof manager.stopSession === "function"
        ? await manager.stopSession()
        : null;
    if (!stoppedState) {
      await manager.cleanup?.();
    }
    context.needsCleanup = false;
    context.started = false;
    return {
      needed: true,
      status: "completed",
      state: stoppedState ? sanitizeState(stoppedState, request) : safeManagerState(manager, request),
    };
  } catch (error) {
    return {
      needed: true,
      status: "failed",
      error: errorMessage(error),
      state: safeManagerState(manager, request),
    };
  }
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
    json: false,
    raw: false,
    pretty: false,
    noOverlay: false,
    overlayHoldMs: 900,
    traceTail: 3,
    includePerception: true,
    perceptionLimit: 40,
    confirmRisk: null,
    planOnly: false,
    shorthandActions: [],
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
      case "--start":
        request.shorthandActions.push({ type: "start" });
        break;
      case "--observe":
        request.shorthandActions.push({ type: "observe" });
        break;
      case "--stop":
        request.shorthandActions.push({ type: "stop" });
        break;
      case "--move":
        request.shorthandActions.push({
          type: "move",
          ...parseCliPoint(requireValue(args, ++index, arg), arg),
        });
        break;
      case "--click":
        request.shorthandActions.push({
          type: "click",
          ...parseCliPoint(requireValue(args, ++index, arg), arg),
        });
        break;
      case "--double-click":
      case "--doubleClick":
        request.shorthandActions.push({
          type: "doubleClick",
          ...parseCliPoint(requireValue(args, ++index, arg), arg),
        });
        break;
      case "--right-click":
      case "--rightClick":
        request.shorthandActions.push({
          type: "rightClick",
          ...parseCliPoint(requireValue(args, ++index, arg), arg),
        });
        break;
      case "--scroll":
        request.shorthandActions.push(
          parseCliScroll(requireValue(args, ++index, arg), arg),
        );
        break;
      case "--find-text":
      case "--findText":
        request.shorthandActions.push({
          type: "findText",
          text: requireValue(args, ++index, arg),
        });
        break;
      case "--click-text":
      case "--clickText":
        request.shorthandActions.push({
          type: "clickText",
          text: requireValue(args, ++index, arg),
        });
        break;
      case "--set-text":
      case "--setText":
        request.shorthandActions.push(
          parseCliSetText(requireValue(args, ++index, arg), arg),
        );
        break;
      case "--type":
      case "--text":
        request.shorthandActions.push({
          type: "type",
          text: requireValue(args, ++index, arg),
        });
        break;
      case "--key":
        request.shorthandActions.push(
          parseCliKey(requireValue(args, ++index, arg), arg),
        );
        break;
      case "--hotkey":
        request.shorthandActions.push(
          parseCliHotkey(requireValue(args, ++index, arg), arg),
        );
        break;
      case "--drag":
        request.shorthandActions.push(
          parseCliDrag(requireValue(args, ++index, arg), arg),
        );
        break;
      case "--wait":
      case "--pause":
        request.shorthandActions.push({
          type: "wait",
          ms: parseWaitMs(requireValue(args, ++index, arg), arg),
        });
        break;
      case "--json":
        request.json = true;
        break;
      case "--raw":
        request.raw = true;
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
      case "--include-perception":
        request.includePerception = true;
        break;
      case "--no-perception":
        request.includePerception = false;
        break;
      case "--perception-limit":
        request.perceptionLimit = parseNonNegativeInteger(
          requireValue(args, ++index, arg),
          arg,
        );
        break;
      case "--confirm-risk":
        request.confirmRisk = parseConfirmRisk(requireValue(args, ++index, arg), arg);
        break;
      case "--plan-only":
      case "--dry-run":
        request.planOnly = true;
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
      case "--trace-tail":
        request.traceTail = parsePositiveInteger(
          requireValue(args, ++index, arg),
          arg,
        );
        break;
      default:
        throw new Error(`Unsupported argument: ${arg}`);
    }
  }

  if (command === "repl") {
    if (request.actions || request.shorthandActions.length > 0) {
      throw new Error("Command repl does not accept batch action flags.");
    }
    delete request.shorthandActions;
    return request;
  }
  if (command === "run") {
    if (request.actions && request.shorthandActions.length > 0) {
      throw new Error("Use either --actions or shorthand action flags, not both.");
    }
    request.actions = normalizeCliActions(
      request.shorthandActions.length > 0
        ? compileShorthandActions(request.shorthandActions)
        : request.actions ?? DEFAULT_ACTIONS,
    );
    delete request.shorthandActions;
    return request;
  }
  if (["start", "observe", "stop"].includes(command)) {
    if (request.actions || request.shorthandActions.length > 0) {
      throw new Error(`Command ${command} does not accept action flags.`);
    }
    request.command = "run";
    request.actions = [{ type: command }];
    delete request.shorthandActions;
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
      includePerception: request.includePerception,
      perceptionLimit: request.perceptionLimit,
      safety: {
        operationBoundary:
          request.command === "repl"
            ? "computer-use-repl-session"
            : "computer-use-run-batch",
        confirmRisk: request.confirmRisk,
        planOnly: request.planOnly,
      },
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
  if (action.type === "pause") {
    return { ...action, type: "wait" };
  }
  return action;
}

function compileShorthandActions(actions) {
  const compiled = [...actions];
  if (compiled[0]?.type !== "start") {
    compiled.unshift({ type: "start" });
  }
  if (!compiled.some((action) => action.type === "stop")) {
    compiled.push({ type: "stop" });
  }
  return compiled;
}

function parseCliPoint(value, flag) {
  const match = /^([^,]+),([^,]+)$/.exec(value.trim());
  if (!match) {
    throw new Error(`${flag} requires coordinates as x,y`);
  }
  const x = Number(match[1]);
  const y = Number(match[2]);
  if (!Number.isFinite(x) || !Number.isFinite(y)) {
    throw new Error(`${flag} requires finite x,y coordinates`);
  }
  return { x, y };
}

function parseCliDrag(value, flag) {
  const [from, to, extra] = value.split(":");
  if (!from || !to || extra !== undefined) {
    throw new Error(`${flag} requires coordinates as x1,y1:x2,y2`);
  }
  return {
    type: "drag",
    from: parseCliPoint(from, flag),
    to: parseCliPoint(to, flag),
  };
}

function parseCliScroll(value, flag) {
  const [point, delta, extra] = value.split(":");
  if (!point || !delta || extra !== undefined) {
    throw new Error(`${flag} requires x,y:deltaX,deltaY`);
  }
  const [rawDeltaX, rawDeltaY, deltaExtra] = delta.split(",");
  if (rawDeltaX === undefined || rawDeltaY === undefined || deltaExtra !== undefined) {
    throw new Error(`${flag} requires x,y:deltaX,deltaY`);
  }
  const deltaX = Number(rawDeltaX);
  const deltaY = Number(rawDeltaY);
  if (!Number.isFinite(deltaX) || !Number.isFinite(deltaY)) {
    throw new Error(`${flag} requires finite deltaX,deltaY`);
  }
  return {
    type: "scroll",
    ...parseCliPoint(point, flag),
    deltaX,
    deltaY,
  };
}

function parseCliSetText(value, flag) {
  const index = value.indexOf("=");
  if (index <= 0) {
    throw new Error(`${flag} requires query=text`);
  }
  const query = value.slice(0, index).trim();
  const text = value.slice(index + 1);
  if (!query) {
    throw new Error(`${flag} requires a non-empty query`);
  }
  return { type: "setText", query, text };
}

function parseCliKey(value, flag) {
  const parts = value
    .split("+")
    .map((part) => part.trim())
    .filter(Boolean);
  if (parts.length === 0) {
    throw new Error(`${flag} requires a key name`);
  }
  const key = parts.at(-1);
  const modifiers = parts.slice(0, -1).map(normalizeCliModifier);
  return { type: "key", key, modifiers };
}

function parseCliHotkey(value, flag) {
  const parsed = parseCliKey(value, flag);
  return { ...parsed, type: "hotkey" };
}

function parseWaitMs(value, flag) {
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < 0 || parsed > 10_000) {
    throw new Error(`${flag} requires milliseconds between 0 and 10000`);
  }
  return parsed;
}

function parseReplLine(line) {
  const trimmed = line.trim();
  if (!trimmed || trimmed.startsWith("#")) {
    return { kind: "empty" };
  }
  if (trimmed.startsWith("{")) {
    return { kind: "action", action: JSON.parse(trimmed) };
  }
  const tokens = splitReplCommandLine(trimmed);
  const command = normalizeReplCommand(tokens.shift() ?? "");
  switch (command) {
    case "exit":
    case "quit":
      return { kind: "exit" };
    case "help":
      return { kind: "help" };
    case "status":
      return { kind: "status" };
    case "trace":
      return {
        kind: "trace",
        count: tokens[0] ? parsePositiveInteger(tokens[0], "trace") : null,
      };
    case "json":
      return { kind: "mode", mode: "json", value: parseReplModeValue(tokens) };
    case "raw":
      return { kind: "mode", mode: "raw", value: parseReplModeValue(tokens) };
    case "start":
      return { kind: "action", action: parseReplStart(tokens) };
    case "observe":
    case "stop":
      requireNoReplArgs(command, tokens);
      return { kind: "action", action: { type: command } };
    case "move":
    case "click":
    case "doubleClick":
    case "rightClick":
      return {
        kind: "action",
        action: {
          type: command,
          ...parseCliPoint(requireReplArg(command, tokens, 0), command),
        },
      };
    case "scroll":
      return { kind: "action", action: parseReplScroll(tokens) };
    case "findText":
      return { kind: "action", action: { type: "findText", text: tokens.join(" ") } };
    case "clickText":
      return { kind: "action", action: { type: "clickText", text: tokens.join(" ") } };
    case "setText":
      return {
        kind: "action",
        action: parseCliSetText(tokens.join(" "), command),
      };
    case "key":
      return {
        kind: "action",
        action: parseCliKey(requireReplArg(command, tokens, 0), command),
      };
    case "hotkey":
      return {
        kind: "action",
        action: parseCliHotkey(requireReplArg(command, tokens, 0), command),
      };
    case "type":
      return { kind: "action", action: { type: "type", text: tokens.join(" ") } };
    case "drag":
      return {
        kind: "action",
        action: parseCliDrag(requireReplArg(command, tokens, 0), command),
      };
    case "wait":
    case "pause":
      return {
        kind: "action",
        action: {
          type: "wait",
          ms: parseWaitMs(requireReplArg(command, tokens, 0), command),
        },
      };
    default:
      throw new Error(`Unsupported Computer Use REPL command: ${command || trimmed}`);
  }
}

function normalizeReplCommand(command) {
  switch (command) {
    case "double-click":
      return "doubleClick";
    case "right-click":
      return "rightClick";
    case "find-text":
      return "findText";
    case "click-text":
      return "clickText";
    case "set-text":
      return "setText";
    default:
      return command;
  }
}

function parseReplStart(tokens) {
  if (tokens.length === 0) {
    return { type: "start" };
  }
  if (tokens[0] === "--app" || tokens[0] === "--target-app") {
    return { type: "start", app: requireReplArg("start", tokens, 1) };
  }
  if (tokens.length === 1) {
    return { type: "start", app: tokens[0] };
  }
  throw new Error("start accepts at most one app target");
}

function parseReplScroll(tokens) {
  const point = requireReplArg("scroll", tokens, 0);
  if (point.includes(":")) {
    return parseCliScroll(point, "scroll");
  }
  const delta = requireReplArg("scroll", tokens, 1);
  return parseCliScroll(`${point}:${delta}`, "scroll");
}

function parseReplModeValue(tokens) {
  const value = tokens[0] ?? "on";
  switch (value.toLowerCase()) {
    case "on":
    case "true":
    case "1":
      return true;
    case "off":
    case "false":
    case "0":
      return false;
    default:
      throw new Error(`Mode value must be on or off, got ${value}`);
  }
}

function splitReplCommandLine(value) {
  const tokens = [];
  let token = "";
  let quote = null;
  let escaping = false;
  for (const char of value) {
    if (escaping) {
      token += char;
      escaping = false;
      continue;
    }
    if (char === "\\") {
      escaping = true;
      continue;
    }
    if (quote) {
      if (char === quote) {
        quote = null;
      } else {
        token += char;
      }
      continue;
    }
    if (char === "'" || char === "\"") {
      quote = char;
      continue;
    }
    if (/\s/.test(char)) {
      if (token.length > 0) {
        tokens.push(token);
        token = "";
      }
      continue;
    }
    token += char;
  }
  if (escaping) {
    token += "\\";
  }
  if (quote) {
    throw new Error("Unterminated quote in REPL command");
  }
  if (token.length > 0) {
    tokens.push(token);
  }
  return tokens;
}

function requireNoReplArgs(command, tokens) {
  if (tokens.length > 0) {
    throw new Error(`${command} does not accept arguments`);
  }
}

function requireReplArg(command, tokens, index) {
  const value = tokens[index];
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`${command} requires argument ${index + 1}`);
  }
  return value;
}

function normalizeCliModifier(value) {
  switch (value.toLowerCase()) {
    case "command":
    case "meta":
      return "cmd";
    case "control":
      return "ctrl";
    case "option":
      return "alt";
    default:
      return value.toLowerCase();
  }
}

function completedResult(index, action, state, request) {
  return {
    index,
    action,
    status: "completed",
    state: sanitizeState(state, request),
  };
}

function managerActionResult(index, action, state, request, traceItem = null) {
  if (traceItem?.status === "blocked") {
    return {
      index,
      action,
      status: "blocked",
      policy: traceItem.policy ?? null,
      evidence: traceItem.evidence ?? null,
      audit: traceItem.audit ?? null,
      state: sanitizeState(state, request),
    };
  }
  if (traceItem?.status === "failed") {
    return {
      index,
      action,
      status: "failed",
      policy: traceItem.policy ?? null,
      evidence: traceItem.evidence ?? null,
      audit: traceItem.audit ?? null,
      error: traceItem.error ?? null,
      state: sanitizeState(state, request),
    };
  }
  return {
    index,
    action,
    status: "completed",
    policy: traceItem?.policy ?? null,
    evidence: traceItem?.evidence ?? null,
    audit: traceItem?.audit ?? null,
    state: sanitizeState(state, request),
  };
}

function blockedResult(index, action, state, request, policy = null) {
  return {
    index,
    action,
    status: "blocked",
    policy:
      policy ?? {
        kind: "disabled",
        allowed: false,
        reason:
          "This Computer Use CLI action is not supported by the current run command.",
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

function managerTraceCursor(manager) {
  try {
    const trace = manager.state?.()?.trace;
    if (!Array.isArray(trace)) {
      return { length: 0, lastSequence: null };
    }
    const lastSequence = trace.at(-1)?.sequence;
    return {
      length: trace.length,
      lastSequence: Number.isFinite(lastSequence) ? lastSequence : null,
    };
  } catch {
    return { length: 0, lastSequence: null };
  }
}

function newTraceItemForAction(state, action, traceCursor) {
  const trace = Array.isArray(state?.trace) ? state.trace : [];
  const newTrace = Number.isFinite(traceCursor?.lastSequence)
    ? trace.filter((item) => Number.isFinite(item?.sequence) && item.sequence > traceCursor.lastSequence)
    : trace.slice(traceCursor?.length ?? 0);
  const traceItem = newTrace.findLast?.((item) => item?.action?.type === action.type) ?? null;
  if (traceItem?.action?.type !== action.type) {
    return null;
  }
  return traceItem;
}

function sanitizeState(state, request) {
  const copy = JSON.parse(JSON.stringify(state ?? null));
  if (!copy) {
    return copy;
  }
  stripScreenshotData(copy.observation?.screenshot, request);
  stripScreenshotData(
    copy.observation?.perception?.windowCrop?.screenshot,
    request,
  );
  for (const item of copy.trace ?? []) {
    stripScreenshotData(item.screenshot, request);
  }
  return copy;
}

function sanitizeTraceItem(traceItem, request) {
  if (!traceItem) {
    return null;
  }
  const copy = JSON.parse(JSON.stringify(traceItem));
  stripScreenshotData(copy.screenshot, request);
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

function formatReplActionResult({ sessionId, action, result, traceItem = null }, settings) {
  const state = result.state ?? null;
  const trace = traceItem;
  const payload = {
    type: "action",
    sessionId,
    index: result.index,
    action,
    status: result.status,
    policy: result.policy ?? trace?.policy ?? null,
    error: result.error ?? trace?.error ?? null,
    evidence: result.evidence ?? trace?.evidence ?? null,
    audit: result.audit ?? trace?.audit ?? null,
    targetVisibility: state?.targetVisibility ?? null,
    traceTail: traceTailFromState(state, settings.traceTail),
  };
  if (settings.raw) {
    payload.state = state;
  } else {
    payload.state = summarizeState(state);
  }
  return formatReplPayload(payload, settings, () => {
    const evidence = payload.evidence
      ? ` evidence=${truncate(JSON.stringify(payload.evidence), 500)}`
      : "";
    const error = payload.error ? ` error=${truncate(payload.error, 240)}` : "";
    return `${payload.status} action=${action.type} policy=${payload.policy?.kind ?? "none"} target=${payload.targetVisibility ?? "unknown"}${evidence}${error}`;
  });
}

function formatReplStateResult({ sessionId, command, state }, settings) {
  const payload = {
    type: command,
    sessionId,
    state: settings.raw ? state : summarizeState(state),
    traceTail: traceTailFromState(state, settings.traceTail),
  };
  return formatReplPayload(payload, settings, () => {
    const summary = payload.state ?? {};
    const perception = summary.perception
      ? ` perception=elements:${summary.perception.elementCount ?? 0},crop:${summary.perception.windowCrop ? "yes" : "no"},limits:${summary.perception.limitationCount ?? 0}`
      : "";
    return `status=${summary.status ?? "none"} target=${summary.target?.app ?? "none"} visibility=${summary.targetVisibility ?? "unknown"} trace=${summary.traceCount ?? 0}${perception}`;
  });
}

function formatReplTraceResult({ sessionId, command, state, count }, settings) {
  const payload = {
    type: command,
    sessionId,
    traceTail: traceTailFromState(state, count),
  };
  return formatReplPayload(payload, settings, () =>
    JSON.stringify(payload.traceTail),
  );
}

function formatReplControlResult(payload, settings) {
  return formatReplPayload({ type: "control", ...payload }, settings, () => {
    if (payload.command === "help") {
      return payload.help;
    }
    if (payload.command === "exit") {
      return `exit cleanup=${payload.cleanup?.status ?? "none"}`;
    }
    if (payload.command === "cleanup") {
      const error = payload.error ? ` error=${truncate(payload.error, 240)}` : "";
      return `cleanup reason=${payload.reason ?? "unknown"} status=${payload.cleanup?.status ?? "none"}${error}`;
    }
    if (payload.command === "json" || payload.command === "raw") {
      return `${payload.command}=${payload[payload.command] ? "on" : "off"}`;
    }
    return `${payload.command ?? "control"} ok`;
  });
}

function formatReplPayload(payload, settings, humanFormatter) {
  if (settings.json) {
    return JSON.stringify(payload, null, settings.pretty ? 2 : 0);
  }
  return humanFormatter();
}

function summarizeState(state) {
  if (!state) {
    return null;
  }
  return {
    status: state.status ?? null,
    target: state.target ?? null,
    targetVisibility: state.targetVisibility ?? null,
    pendingAction: state.pendingAction ?? null,
    policy: state.policy ?? null,
    overlay: state.overlay
      ? {
          mode: state.overlay.mode ?? null,
          visible: state.overlay.visible === true,
          reason: state.overlay.reason ?? null,
        }
      : null,
    cursor: state.cursor ?? null,
    agentCursor: state.agentCursor ?? null,
    systemCursor: state.systemCursor ?? null,
    observationSequence: state.observation?.sequence ?? null,
    perception: summarizePerception(state.observation?.perception),
    traceCount: Array.isArray(state.trace) ? state.trace.length : 0,
  };
}

function summarizePerception(perception) {
  if (!perception) {
    return null;
  }
  return {
    enabled: perception.enabled === true,
    source: perception.source ?? null,
    windowCrop: perception.windowCrop
      ? {
          bounds: perception.windowCrop.bounds ?? null,
          hasScreenshot: Boolean(perception.windowCrop.screenshot),
        }
      : null,
    elementCount: Array.isArray(perception.accessibilityElements)
      ? perception.accessibilityElements.length
      : 0,
    limitationCount: Array.isArray(perception.limitations)
      ? perception.limitations.length
      : 0,
  };
}

function traceTailFromState(state, requestedCount) {
  const trace = Array.isArray(state?.trace) ? state.trace : [];
  const count = Math.min(Math.max(Number(requestedCount) || 3, 1), 20);
  return trace.slice(-count).map((item) => ({
    sequence: item.sequence ?? null,
    action: item.action ?? null,
    status: item.status ?? null,
    policy: item.policy ?? null,
    error: item.error ?? null,
    evidence: item.evidence ?? null,
    audit: item.audit ?? null,
    activation: item.activation ?? null,
  }));
}

function truncate(value, maxLength) {
  if (typeof value !== "string" || value.length <= maxLength) {
    return value;
  }
  return `${value.slice(0, maxLength - 1)}…`;
}

function cliPolicy(request = {}) {
  return {
    version: "computer-use-cli-v2",
    sessionMode: "single-process-batch",
    operationBoundary:
      request.command === "repl"
        ? "computer-use-repl-session"
        : "computer-use-run-batch",
    planOnly: request.planOnly === true,
    confirmRisk: request.confirmRisk ?? null,
    highRiskRequiresConfirmation: true,
    allowedActions: [
      "start",
      "observe",
      "move",
      "click",
      "doubleClick",
      "rightClick",
      "scroll",
      "findText",
      "clickText",
      "setText",
      "key",
      "hotkey",
      "type",
      "drag",
      "wait",
      "pause",
      "stop",
    ],
    disabledActions: [],
    moveDoesNotMoveSystemCursor: true,
  };
}

function cliLimitations() {
  return [
    "The CLI keeps session state only for the lifetime of one `run` process.",
    "Click, doubleClick, rightClick, scroll, key, hotkey, type, and drag are real desktop side effects and require a matched target plus Accessibility permission.",
    "Background targets are activated by the Computer Use session, then re-observed before any side-effect action is sent.",
    "setText is a semantic Accessibility set-value action for one unique writable target element; it does not send background keyboard events.",
    "Screenshots include a bounded data URL by default; pass --omit-screenshot-data for metadata-only output.",
    "Target observations include bounded perception facts by default when macOS exposes real window/AX evidence; pass --no-perception to disable AX/window crop extraction.",
    "Visible agent cursor feedback is target-bound. Background targets are not drawn over unrelated foreground apps before activation.",
    "After a visible move, the CLI waits --overlay-hold-ms before the next action so the cursor can be seen.",
    "High-risk actions require --confirm-risk high; --plan-only blocks real side effects before native input.",
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

function parsePositiveInteger(value, flag) {
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < 1) {
    throw new Error(`${flag} requires a positive integer`);
  }
  return parsed;
}

function parseConfirmRisk(value, flag) {
  const normalized = String(value ?? "").toLowerCase();
  if (["high", "all"].includes(normalized)) {
    return normalized;
  }
  throw new Error(`${flag} requires high or all`);
}

async function delay(ms) {
  if (!Number.isSafeInteger(ms) || ms <= 0) {
    return;
  }
  await new Promise((resolve) => setTimeout(resolve, ms));
}

function replHelp() {
  return [
    "Computer Use REPL commands:",
    "  start [--app <bundle-or-name>] | observe | status | trace [count] | stop | exit",
    "  move x,y | click x,y | double-click x,y | right-click x,y | scroll x,y deltaX,deltaY",
    "  find-text <text> | click-text <text> | set-text query=value",
    "  key key|mod+key | hotkey mod+key | type <text> | drag x1,y1:x2,y2 | wait ms | pause ms",
    "  json on|off | raw on|off | help",
    "  JSON action lines are also accepted, for example: {\"type\":\"move\",\"x\":420,\"y\":360}",
  ].join("\n");
}

function usage() {
  return [
    "Usage:",
    "  node scripts/morpheus-computer-use.mjs run --app <bundle-or-name> --json --actions '<json-array>'",
    "  node scripts/morpheus-computer-use.mjs run --app <bundle-or-name> --click 300,230 --type 'hello' --hotkey cmd+s",
    "  node scripts/morpheus-computer-use.mjs repl --app <bundle-or-name> --omit-screenshot-data",
    "",
    "Actions:",
    "  start, observe, move, click, doubleClick, rightClick, scroll, findText, clickText, setText, key, hotkey, type, drag, wait/pause, stop",
    "",
    "Shorthand flags:",
    "  --start --observe --stop",
    "  --move x,y --click x,y --double-click x,y --right-click x,y --scroll x,y:deltaX,deltaY",
    "  --find-text text --click-text text --set-text query=value --type text --text text --key key|mod+key --hotkey mod+key --drag x1,y1:x2,y2 --wait ms --pause ms",
    "",
    "Example:",
    "  node scripts/morpheus-computer-use.mjs run --app com.apple.finder --json --actions '[{\"type\":\"start\"},{\"type\":\"observe\"},{\"type\":\"move\",\"x\":420,\"y\":360},{\"type\":\"stop\"}]'",
    "  node scripts/morpheus-computer-use.mjs run --app com.apple.finder --json --move 420,360 --observe",
    "",
    "Options:",
    "  --omit-screenshot-data  Return screenshot metadata without the bounded data URL.",
    "  --no-perception         Disable foreground window crop and AX element extraction.",
    "  --perception-limit <n>  Bound returned AX element candidates (default: 40, max: 80).",
    "  --confirm-risk high     Explicitly allow high-risk side effects such as destructive shortcuts or sensitive text.",
    "  --plan-only             Preflight actions but block real side effects before native input.",
    "  --json                  For repl, emit one JSON event per command.",
    "  --raw                   For repl JSON output, include sanitized full state.",
    "  --trace-tail <n>        Include at most n recent trace items in repl output (bounded to 20).",
    "  --overlay-hold-ms <ms>  Keep a visible moved cursor on screen before the next action (default: 900).",
    "",
    replHelp(),
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

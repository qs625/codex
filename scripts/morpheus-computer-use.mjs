#!/usr/bin/env node

import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const {
  createComputerUseManager,
} = require("../apps/root-worker-prototype/electron/computerUse.cjs");

const CLI_DISABLED_ACTIONS = new Set(["click", "type", "key", "drag"]);
const DEFAULT_ACTIONS = [{ type: "start" }];

export async function runComputerUseCli({
  argv = process.argv.slice(2),
  managerFactory = () => createComputerUseManager(),
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
    const result = await runComputerUseRequest(request, managerFactory);
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
  const manager = managerFactory();
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
        continue;
      }
      throw new Error(`Unsupported Computer Use CLI action: ${action.type}`);
    }
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
  ];
}

function requireValue(args, index, flag) {
  const value = args[index];
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`${flag} requires a value`);
  }
  return value;
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

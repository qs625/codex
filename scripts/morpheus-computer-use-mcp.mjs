#!/usr/bin/env node

import { createRequire } from "node:module";
import { createInterface } from "node:readline";
import { dirname } from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const {
  createComputerUseManager,
} = require(resolveComputerUseManagerModule());

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

const SERVER_INFO = {
  name: "morpheus-computer-use",
  version: "0.1.0",
};
const PROTOCOL_VERSION = "2025-06-18";
const DEFAULT_SESSION_ID = "default";
const SIDE_EFFECT_TOOLS = new Set(["computer.act"]);
const TEXT_ACTION_TOOLS = new Map([
  ["computer.find_text", "findText"],
]);

const TOOL_DEFINITIONS = [
  {
    name: "computer.start_session",
    description:
      "Start a Computer Use session bound to an optional target app and return typed observation evidence.",
    inputSchema: objectSchema({
      app: stringSchema("Bundle identifier or app name to target."),
      includeScreenshotData: booleanSchema(
        "Include bounded screenshot data URLs in the tool result. Defaults to false.",
      ),
      includePerception: booleanSchema(
        "Include bounded Accessibility/window perception facts. Defaults to true.",
      ),
      perceptionLimit: integerSchema(
        "Maximum returned perception elements, from 0 to 80.",
        0,
        80,
      ),
      confirmRisk: enumSchema(
        ["high", "all"],
        "Risk confirmation level for high-risk actions in this session.",
      ),
      planOnly: booleanSchema(
        "Preflight side effects but block native input. Defaults to false.",
      ),
      sessionId: stringSchema(
        "Optional stable session id. Defaults to the server's default session.",
      ),
    }),
  },
  {
    name: "computer.observe",
    description:
      "Observe the current Computer Use target and return screenshot, target, permission, and perception evidence.",
    inputSchema: objectSchema({
      sessionId: stringSchema("Session id returned by computer.start_session."),
      app: stringSchema("Optional app target if the session must be created first."),
      includeScreenshotData: booleanSchema(
        "Include bounded screenshot data URLs in the tool result. Defaults to false.",
      ),
      includePerception: booleanSchema(
        "Include bounded Accessibility/window perception facts. Defaults to true.",
      ),
      perceptionLimit: integerSchema(
        "Maximum returned perception elements, from 0 to 80.",
        0,
        80,
      ),
    }),
  },
  {
    name: "computer.find_text",
    description:
      "Find visible text in the current Computer Use observation. Blocks with typed policy evidence when Accessibility is not trusted.",
    inputSchema: objectSchema(
      {
        text: stringSchema("Visible text to find."),
        maxMatches: integerSchema("Maximum matches to return, from 1 to 20.", 1, 20),
        sessionId: stringSchema("Session id returned by computer.start_session."),
        includeScreenshotData: booleanSchema(
          "Include bounded screenshot data URLs in the tool result. Defaults to false.",
        ),
      },
      ["text"],
    ),
  },
  {
    name: "computer.act",
    description:
      "Run a typed Computer Use action through the shared safety policy, permission gate, and audit trail.",
    inputSchema: objectSchema(
      {
        action: {
          type: "object",
          description:
            "Computer Use action, for example move/click/type/key/hotkey/drag/wait/clickText/setText.",
          additionalProperties: true,
        },
        sessionId: stringSchema("Session id returned by computer.start_session."),
        includeScreenshotData: booleanSchema(
          "Include bounded screenshot data URLs in the tool result. Defaults to false.",
        ),
      },
      ["action"],
    ),
  },
  {
    name: "computer.stop",
    description:
      "Stop a Computer Use session and clean up temporary screenshot/overlay state.",
    inputSchema: objectSchema({
      sessionId: stringSchema("Session id returned by computer.start_session."),
      includeScreenshotData: booleanSchema(
        "Include bounded screenshot data URLs in the tool result. Defaults to false.",
      ),
    }),
  },
  {
    name: "computer.permissions_status",
    description:
      "Report the Computer Use MCP helper permission subject and optional read-only observation status.",
    inputSchema: objectSchema({
      app: stringSchema("Optional app target to use for the diagnostic observation."),
      includeObservation: booleanSchema(
        "Run a read-only observe to report Screen Recording and Accessibility evidence. Defaults to true.",
      ),
      includeScreenshotData: booleanSchema(
        "Include bounded screenshot data URLs in the diagnostic result. Defaults to false.",
      ),
      includePerception: booleanSchema(
        "Include bounded Accessibility/window perception facts. Defaults to true.",
      ),
      perceptionLimit: integerSchema(
        "Maximum returned perception elements, from 0 to 80.",
        0,
        80,
      ),
    }),
  },
];

export function createComputerUseMcpServer(options = {}) {
  const managerFactory = options.managerFactory ?? defaultManagerFactory;
  const sessions = new Map();
  const diagnosticsFactory =
    options.diagnosticsFactory ?? (() => defaultPermissionDiagnostics());

  async function callTool(name, args = {}) {
    if (name === "computer.permissions_status") {
      return formatToolResult(
        await permissionsStatus({ args, managerFactory, diagnosticsFactory }),
      );
    }

    if (name === "computer.start_session") {
      const session = await createOrReplaceSession(args, managerFactory, sessions);
      const state = await session.manager.startSession({ app: args.app ?? undefined });
      return formatToolResult({
        ok: true,
        tool: name,
        sessionId: session.id,
        status: "completed",
        permissions: permissionsFromState(state),
        state: sanitizeState(state, args),
      });
    }

    if (name === "computer.observe") {
      const session = await getOrCreateSession(args, managerFactory, sessions);
      const state = await session.manager.observe("mcp");
      return formatToolResult({
        ok: true,
        tool: name,
        sessionId: session.id,
        status: "completed",
        permissions: permissionsFromState(state),
        state: sanitizeState(state, args),
      });
    }

    if (TEXT_ACTION_TOOLS.has(name)) {
      const actionType = TEXT_ACTION_TOOLS.get(name);
      const action = {
        type: actionType,
        text: requireString(args.text, "text"),
        maxMatches: args.maxMatches,
      };
      return formatToolResult(
        await runActionTool({ name, action, args, managerFactory, sessions }),
      );
    }

    if (SIDE_EFFECT_TOOLS.has(name)) {
      return formatToolResult(
        await runActionTool({
          name,
          action: requireObject(args.action, "action"),
          args,
          managerFactory,
          sessions,
        }),
      );
    }

    if (name === "computer.stop") {
      const id = sessionIdFromArgs(args);
      const session = sessions.get(id);
      if (!session) {
        return formatToolResult({
          ok: true,
          tool: name,
          sessionId: id,
          status: "skipped",
          state: null,
        });
      }
      const state = await session.manager.stopSession();
      await session.manager.cleanup?.();
      sessions.delete(id);
      return formatToolResult({
        ok: true,
        tool: name,
        sessionId: id,
        status: "completed",
        permissions: permissionsFromState(state),
        state: sanitizeState(state, args),
      });
    }

    throw new Error(`Unsupported Computer Use MCP tool: ${name}`);
  }

  async function handleJsonRpc(message) {
    if (message.method === "initialize") {
      return {
        protocolVersion: PROTOCOL_VERSION,
        serverInfo: SERVER_INFO,
        capabilities: { tools: {} },
      };
    }
    if (message.method === "tools/list") {
      return { tools: TOOL_DEFINITIONS };
    }
    if (message.method === "tools/call") {
      const params = message.params ?? {};
      return await callTool(params.name, params.arguments ?? {});
    }
    if (message.method === "notifications/initialized") {
      return undefined;
    }
    throw jsonRpcError(-32601, `Unsupported method: ${message.method}`);
  }

  async function close() {
    for (const session of sessions.values()) {
      await session.manager.cleanup?.();
    }
    sessions.clear();
  }

  return {
    callTool,
    close,
    handleJsonRpc,
    listTools: () => TOOL_DEFINITIONS,
  };
}

export async function runComputerUseMcpServer({
  input = process.stdin,
  output = process.stdout,
  server = createComputerUseMcpServer(),
} = {}) {
  const lines = createInterface({ input, crlfDelay: Infinity });
  try {
    for await (const line of lines) {
      if (!line.trim()) {
        continue;
      }
      let message;
      try {
        message = JSON.parse(line);
      } catch (error) {
        writeJson(output, {
          jsonrpc: "2.0",
          id: null,
          error: serializeJsonRpcError(jsonRpcError(-32700, errorMessage(error))),
        });
        continue;
      }
      if (!Object.hasOwn(message, "id")) {
        await server.handleJsonRpc(message);
        continue;
      }
      try {
        const result = await server.handleJsonRpc(message);
        writeJson(output, { jsonrpc: "2.0", id: message.id, result });
      } catch (error) {
        const rpcError =
          error && typeof error === "object" && Number.isInteger(error.code)
            ? error
            : jsonRpcError(-32000, errorMessage(error));
        writeJson(output, {
          jsonrpc: "2.0",
          id: message.id,
          error: serializeJsonRpcError(rpcError),
        });
      }
    }
  } finally {
    await server.close?.();
  }
}

async function permissionsStatus({ args, managerFactory, diagnosticsFactory }) {
  const diagnostics = diagnosticsFactory();
  const includeObservation = args.includeObservation !== false;
  if (!includeObservation) {
    return {
      ok: true,
      tool: "computer.permissions_status",
      status: "completed",
      permissions: {
        screenRecording: "unknown",
        accessibilityTrusted: "unknown",
      },
      diagnostics,
      limitations: permissionLimitations(diagnostics),
    };
  }

  const session = await newSession(args, managerFactory);
  try {
    const state = await session.manager.startSession({ app: args.app ?? undefined });
    return {
      ok: true,
      tool: "computer.permissions_status",
      sessionId: session.id,
      status: "completed",
      permissions: permissionsFromState(state),
      diagnostics,
      limitations: permissionLimitations(diagnostics),
      state: sanitizeState(state, args),
    };
  } catch (error) {
    return {
      ok: false,
      tool: "computer.permissions_status",
      status: "failed",
      permissions: {
        screenRecording: "unknown",
        accessibilityTrusted: "unknown",
      },
      diagnostics,
      limitations: permissionLimitations(diagnostics),
      error: errorMessage(error),
    };
  } finally {
    await session.manager.cleanup?.();
  }
}

async function runActionTool({ name, action, args, managerFactory, sessions }) {
  const session = await getOrCreateSession(args, managerFactory, sessions);
  const traceCursor = managerTraceCursor(session.manager);
  const state = await session.manager.act(action);
  const traceItem = newTraceItemForAction(state, action, traceCursor);
  return {
    ok: true,
    tool: name,
    sessionId: session.id,
    action,
    status: traceItem?.status ?? "completed",
    policy: traceItem?.policy ?? null,
    evidence: traceItem?.evidence ?? null,
    audit: traceItem?.audit ?? null,
    error: traceItem?.error ?? null,
    permissions: permissionsFromState(state),
    state: sanitizeState(state, args),
  };
}

async function createOrReplaceSession(args, managerFactory, sessions) {
  const id = sessionIdFromArgs(args);
  const existing = sessions.get(id);
  if (existing) {
    await existing.manager.cleanup?.();
    sessions.delete(id);
  }
  const session = await newSession(args, managerFactory, id);
  sessions.set(id, session);
  return session;
}

async function getOrCreateSession(args, managerFactory, sessions) {
  const id = sessionIdFromArgs(args);
  const existing = sessions.get(id);
  if (existing) {
    return existing;
  }
  const session = await newSession(args, managerFactory, id);
  await session.manager.startSession({ app: args.app ?? undefined });
  sessions.set(id, session);
  return session;
}

async function newSession(args, managerFactory, id = sessionIdFromArgs(args)) {
  const manager = await managerFactory({
    includePerception: args.includePerception !== false,
    perceptionLimit: args.perceptionLimit,
    confirmRisk: args.confirmRisk,
    planOnly: args.planOnly,
  });
  return { id, manager };
}

async function defaultManagerFactory(options = {}) {
  return createComputerUseManager({
    ...(process.env.MORPHEUS_COMPUTER_USE_NATIVE_SCRIPT
      ? { scriptPath: process.env.MORPHEUS_COMPUTER_USE_NATIVE_SCRIPT }
      : {}),
    includePerception: options.includePerception,
    perceptionLimit: options.perceptionLimit,
    safety: {
      operationBoundary: "computer-use-mcp-session",
      confirmRisk: options.confirmRisk,
      planOnly: options.planOnly === true,
    },
  });
}

function formatToolResult(result) {
  const summary = {
    ok: result.ok === true,
    tool: result.tool,
    status: result.status,
    sessionId: result.sessionId ?? null,
    permissions: result.permissions ?? null,
    policy: result.policy ?? null,
    evidence: result.evidence ?? null,
    audit: result.audit ?? null,
    error: result.error ?? null,
  };
  return {
    content: [
      {
        type: "text",
        text: JSON.stringify(summary),
      },
    ],
    structuredContent: result,
    isError: result.ok === false && result.status !== "blocked",
  };
}

function permissionsFromState(state) {
  const observation = state?.observation ?? null;
  return {
    screenRecording: observation?.screenshot ? "granted" : "unknown",
    accessibilityTrusted:
      typeof observation?.accessibilityTrusted === "boolean"
        ? observation.accessibilityTrusted
        : "unknown",
    targetVisibility: state?.targetVisibility ?? observation?.targetVisibility ?? "unknown",
  };
}

function defaultPermissionDiagnostics() {
  const helperBundlePath =
    process.env.MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_PATH ?? null;
  const helperExecutable =
    process.env.MORPHEUS_COMPUTER_USE_HELPER_EXECUTABLE ?? null;
  const helperBundleIdentifier =
    process.env.MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_ID ?? null;
  const packagedHelperBundle = Boolean(helperBundleIdentifier && helperBundlePath);
  return {
    contract: "helper-as-mcp-server",
    helperMode: process.env.MORPHEUS_COMPUTER_USE_HELPER_MODE ?? "repo-local-mcp-server",
    mcpServer: {
      entrypoint: __filename,
      cwd: process.cwd(),
      pid: process.pid,
      ppid: process.ppid,
      execPath: process.execPath,
      argv1: process.argv[1] ?? null,
      platform: process.platform,
    },
    permissionSubject: {
      bundleIdentifier:
        helperBundleIdentifier ?? process.env.MORPHEUS_RUNTIME_BUNDLE_ID ?? null,
      bundlePath: helperBundlePath ?? process.env.MORPHEUS_RUNTIME_BUNDLE_PATH ?? null,
      executablePath: helperExecutable ?? process.execPath,
      packagedHelperBundle,
      stablePermissionSubject: false,
      nativeControlSubject: packagedHelperBundle
        ? "delegated-swift-script-and-screencapture"
        : "repo-local-node-process",
      note:
        "Authorize the process macOS presents for Screen Recording and Accessibility. The packaged helper bundle is the intended MCP server identity, but current native desktop calls are still delegated through Swift and screencapture, so the stable TCC permission subject is not yet proven.",
    },
  };
}

function permissionLimitations(diagnostics) {
  const subject = diagnostics.permissionSubject ?? {};
  const limitations = [
    {
      code: "launcher-not-permission-subject",
      message:
        "The outer Launcher supervises Runtime Capsule selection and is not the Computer Use permission subject.",
    },
  ];
  if (!subject.bundleIdentifier && !subject.bundlePath) {
    limitations.push({
      code: "repo-local-helper-not-bundled",
      message:
        "This process is running as a repo-local MCP server without a packaged Computer Use helper bundle.",
    });
  }
  if (subject.packagedHelperBundle && subject.stablePermissionSubject !== true) {
    limitations.push({
      code: "native-permission-subject-not-contained",
      message:
        "The packaged helper bundle starts the MCP server, but native desktop control still delegates to Swift and screencapture, so macOS may require authorization for the delegated runtime/toolchain process until native calls move into a signed helper executable.",
    });
  }
  return limitations;
}

function sanitizeState(state, args = {}) {
  const copy = JSON.parse(JSON.stringify(state ?? null));
  if (!copy) {
    return copy;
  }
  stripScreenshotData(copy.observation?.screenshot, args);
  stripScreenshotData(copy.observation?.perception?.windowCrop?.screenshot, args);
  for (const item of copy.trace ?? []) {
    stripScreenshotData(item.screenshot, args);
  }
  return copy;
}

function stripScreenshotData(screenshot, args = {}) {
  if (!screenshot) {
    return;
  }
  if (Object.hasOwn(screenshot, "path")) {
    delete screenshot.path;
    screenshot.pathOmitted =
      "Temporary screenshot file remains helper-owned and is omitted from MCP output.";
  }
  if (args.includeScreenshotData !== true && Object.hasOwn(screenshot, "dataUrl")) {
    delete screenshot.dataUrl;
    screenshot.dataUrlOmitted = true;
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
    ? trace.filter(
        (item) =>
          Number.isFinite(item?.sequence) && item.sequence > traceCursor.lastSequence,
      )
    : trace.slice(traceCursor?.length ?? 0);
  const traceItem =
    newTrace.findLast?.((item) => item?.action?.type === action.type) ?? null;
  return traceItem?.action?.type === action.type ? traceItem : null;
}

function sessionIdFromArgs(args = {}) {
  return typeof args.sessionId === "string" && args.sessionId.length > 0
    ? args.sessionId
    : DEFAULT_SESSION_ID;
}

function requireObject(value, name) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${name} must be an object`);
  }
  return value;
}

function requireString(value, name) {
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new Error(`${name} must be a non-empty string`);
  }
  return value;
}

function objectSchema(properties, required = []) {
  return {
    type: "object",
    properties,
    required,
    additionalProperties: false,
  };
}

function stringSchema(description) {
  return { type: "string", description };
}

function booleanSchema(description) {
  return { type: "boolean", description };
}

function integerSchema(description, minimum, maximum) {
  return { type: "integer", description, minimum, maximum };
}

function enumSchema(values, description) {
  return { type: "string", enum: values, description };
}

function writeJson(output, value) {
  output.write(`${JSON.stringify(value)}\n`);
}

function jsonRpcError(code, message) {
  const error = new Error(message);
  error.code = code;
  return error;
}

function serializeJsonRpcError(error) {
  return {
    code: Number.isInteger(error.code) ? error.code : -32000,
    message: errorMessage(error),
  };
}

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}

function resolveComputerUseManagerModule() {
  return (
    process.env.MORPHEUS_COMPUTER_USE_MANAGER_MODULE ??
    "../apps/root-worker-prototype/electron/computerUse.cjs"
  );
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  await runComputerUseMcpServer();
}

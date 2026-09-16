const { execFile } = require("node:child_process");
const fs = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");
const { randomUUID } = require("node:crypto");
const { promisify } = require("node:util");

const execFileAsync = promisify(execFile);
const MAX_SCREENSHOT_BYTES = 12 * 1024 * 1024;
const MAX_TRACE_ITEMS = 80;
const DANGEROUS_TEXT_PATTERN =
  /\b(password|passcode|token|secret|delete|remove|send|submit|purchase|buy|transfer|bank|credit|sudo|rm\s+-rf)\b/i;

function createComputerUseManager(options = {}) {
  return new ComputerUseManager(options);
}

class ComputerUseManager {
  constructor(options = {}) {
    this.nativeClient =
      options.nativeClient ??
      createMacNativeComputerUseClient({
        scriptPath:
          options.scriptPath ?? path.join(__dirname, "computerUseMacNative.swift"),
        tmpDir: options.tmpDir,
      });
    this.clock = options.clock ?? (() => Date.now());
    this.session = null;
  }

  async cleanup() {
    await this.nativeClient.cleanup?.();
  }

  async startSession(options = {}) {
    if (!this.session || this.session.status === "stopped") {
      this.session = createEmptySession(this.clock(), options);
    }
    await this.observe("start");
    return this.state();
  }

  async stopSession() {
    if (!this.session) {
      this.session = createEmptySession(this.clock(), {});
    }
    this.session.status = "stopped";
    this.session.updatedAtMs = this.clock();
    await this.nativeClient.cleanup?.();
    return this.state();
  }

  async observe(reason = "manual") {
    this.ensureSession();
    this.session.status = "observing";
    this.session.pendingAction = null;
    this.session.updatedAtMs = this.clock();
    try {
      await this.observeNow(reason);
    } catch (error) {
      this.applyError(error, "observe");
    }
    return this.state();
  }

  async act(action) {
    this.ensureSession();
    const normalized = normalizeAction(action);
    let policy = classifyComputerUseAction(normalized);
    const startedAtMs = this.clock();
    const traceItem = {
      id: randomUUID(),
      sequence: ++this.session.sequence,
      action: normalized,
      policy,
      status: policy.allowed ? "running" : "blocked",
      error: policy.allowed ? null : policy.reason,
      startedAtMs,
      completedAtMs: policy.allowed ? null : startedAtMs,
      observationSequence: this.session.observation?.sequence ?? null,
    };
    appendTrace(this.session, traceItem);
    if (!policy.allowed) {
      this.session.policy = policy;
      this.session.status = "active";
      this.session.pendingAction = null;
      this.session.updatedAtMs = this.clock();
      return this.state();
    }

    try {
      await this.observeNow("pre-action");
    } catch (error) {
      traceItem.status = "failed";
      traceItem.error = errorMessage(error);
      traceItem.completedAtMs = this.clock();
      this.applyError(error, "pre-action");
      return this.state();
    }

    policy = this.policyForAction(normalized);
    traceItem.policy = policy;
    traceItem.observationSequence = this.session.observation?.sequence ?? null;
    if (!policy.allowed) {
      traceItem.status = "blocked";
      traceItem.error = policy.reason;
      traceItem.completedAtMs = this.clock();
      this.session.policy = policy;
      this.session.status = "active";
      this.session.pendingAction = null;
      this.session.updatedAtMs = this.clock();
      return this.state();
    }

    this.session.status = "acting";
    this.session.pendingAction = normalized;
    this.session.policy = policy;
    this.session.updatedAtMs = this.clock();
    try {
      await this.nativeClient.act(normalized);
      traceItem.status = "completed";
      traceItem.completedAtMs = this.clock();
      updatePointerPath(this.session, normalized);
      await this.observe("post-action");
    } catch (error) {
      traceItem.status = "failed";
      traceItem.error = errorMessage(error);
      traceItem.completedAtMs = this.clock();
      this.applyError(error, "action");
    } finally {
      this.session.pendingAction = null;
    }
    return this.state();
  }

  policyForAction(action) {
    const basePolicy = classifyComputerUseAction(action);
    if (!basePolicy.allowed) {
      return basePolicy;
    }
    if (this.session.observation?.accessibilityTrusted === false) {
      return {
        kind: "needs-permission",
        allowed: false,
        reason: "Accessibility permission is required before controlling the desktop.",
      };
    }
    if (!this.session.observation) {
      return {
        kind: "needs-observation",
        allowed: false,
        reason: "Observe the desktop before running Computer Use actions.",
      };
    }
    const mismatch = targetMismatch(this.session.target, this.session.observation?.activeApp);
    if (mismatch) {
      return {
        kind: "target-mismatch",
        allowed: false,
        reason: mismatch,
      };
    }
    return basePolicy;
  }

  state() {
    this.ensureSession();
    return snapshotSession(this.session);
  }

  async observeNow(reason) {
    const observation = await this.nativeClient.observe();
    this.applyObservation(observation, reason);
  }

  ensureSession() {
    if (!this.session) {
      this.session = createEmptySession(this.clock(), {});
    }
  }

  applyObservation(observation, reason) {
    const now = this.clock();
    const previousSequence = this.session.observation?.sequence ?? 0;
    const cursor = normalizePoint(observation.cursor);
    this.session.observation = {
      sequence: previousSequence + 1,
      reason,
      observedAtMs: now,
      cursor,
      activeApp: observation.activeApp ?? null,
      accessibilityTrusted: observation.accessibilityTrusted !== false,
      screenshot: observation.screenshot ?? null,
      error: null,
    };
    if (!this.session.target.app && observation.activeApp) {
      this.session.target.app =
        observation.activeApp.bundleIdentifier || observation.activeApp.name || null;
    }
    this.session.target.window = observation.activeApp?.window ?? null;
    if (cursor) {
      this.session.cursor = cursor;
      pushPointerPoint(this.session, cursor, now, "observe");
    }
    this.session.status = "active";
    this.session.error = null;
    this.session.updatedAtMs = now;
  }

  applyError(error, phase) {
    const now = this.clock();
    this.session.status = "error";
    this.session.error = {
      phase,
      message: errorMessage(error),
      atMs: now,
    };
    this.session.updatedAtMs = now;
  }
}

function createEmptySession(now, options) {
  return {
    id: randomUUID(),
    status: "idle",
    createdAtMs: now,
    updatedAtMs: now,
    target: {
      app: typeof options.app === "string" ? options.app : null,
      window: null,
    },
    sequence: 0,
    observation: null,
    cursor: null,
    pointerPath: [],
    trace: [],
    pendingAction: null,
    policy: null,
    error: null,
  };
}

function snapshotSession(session) {
  return JSON.parse(JSON.stringify(session));
}

function classifyComputerUseAction(action) {
  if (action.type === "observe") {
    return { kind: "read-only", allowed: true, reason: null };
  }
  if (action.type === "drag") {
    return {
      kind: "disabled",
      allowed: false,
      reason: "Drag is reserved for the persistent action boundary but disabled in v1.",
    };
  }
  const modifiers = normalizeModifiers(action.modifiers);
  if (
    (action.type === "type" && DANGEROUS_TEXT_PATTERN.test(action.text ?? "")) ||
    (action.type === "key" &&
      modifiers.includes("cmd") &&
      ["delete", "q", "w"].includes(String(action.key ?? "").toLowerCase()))
  ) {
    return {
      kind: "needs-confirmation",
      allowed: false,
      reason: "Potentially destructive or sensitive action requires a future confirmation boundary.",
    };
  }
  return { kind: "low-risk", allowed: true, reason: null };
}

function normalizeAction(action) {
  if (!action || typeof action !== "object") {
    throw new Error("Computer Use action is required");
  }
  switch (action.type) {
    case "move":
    case "click":
      return { type: action.type, ...requirePoint(action) };
    case "type":
      return { type: "type", text: String(action.text ?? "") };
    case "key":
      return {
        type: "key",
        key: String(action.key ?? ""),
        modifiers: normalizeModifiers(action.modifiers),
      };
    case "drag":
      return {
        type: "drag",
        from: requirePoint(action.from ?? {}),
        to: requirePoint(action.to ?? {}),
      };
    default:
      throw new Error(`Unsupported Computer Use action: ${String(action.type)}`);
  }
}

function requirePoint(value) {
  const x = Number(value.x);
  const y = Number(value.y);
  if (!Number.isFinite(x) || !Number.isFinite(y)) {
    throw new Error("Action requires finite x and y screen coordinates");
  }
  return { x, y };
}

function normalizePoint(value) {
  if (!value || typeof value !== "object") {
    return null;
  }
  const x = Number(value.x);
  const y = Number(value.y);
  if (!Number.isFinite(x) || !Number.isFinite(y)) {
    return null;
  }
  return { x, y };
}

function normalizeModifiers(value) {
  if (!Array.isArray(value)) {
    return [];
  }
  return [
    ...new Set(
      value
        .map((item) => normalizedModifierName(String(item).toLowerCase()))
        .filter(Boolean),
    ),
  ];
}

function normalizedModifierName(value) {
  switch (value) {
    case "command":
    case "meta":
      return "cmd";
    case "control":
      return "ctrl";
    case "option":
      return "alt";
    default:
      return value;
  }
}

function targetMismatch(target, activeApp) {
  if (!target?.app) {
    return null;
  }
  if (!activeApp) {
    return `Target app ${target.app} is not active; no active app was observed.`;
  }
  const expected = normalizeTargetApp(target.app);
  const observed = [
    activeApp.bundleIdentifier,
    activeApp.name,
  ]
    .filter((value) => typeof value === "string" && value.length > 0)
    .map(normalizeTargetApp);
  if (observed.includes(expected)) {
    return null;
  }
  return `Target app ${target.app} is not active; active app is ${activeApp.name || activeApp.bundleIdentifier || "unknown"}.`;
}

function normalizeTargetApp(value) {
  return String(value).trim().toLowerCase();
}

function appendTrace(session, item) {
  session.trace.push(item);
  if (session.trace.length > MAX_TRACE_ITEMS) {
    session.trace.splice(0, session.trace.length - MAX_TRACE_ITEMS);
  }
}

function updatePointerPath(session, action) {
  const now = Date.now();
  if (action.type === "move" || action.type === "click") {
    pushPointerPoint(session, { x: action.x, y: action.y }, now, action.type);
  }
}

function pushPointerPoint(session, point, atMs, source) {
  session.pointerPath.push({ ...point, atMs, source });
  if (session.pointerPath.length > 48) {
    session.pointerPath.splice(0, session.pointerPath.length - 48);
  }
}

function createMacNativeComputerUseClient({ scriptPath, tmpDir } = {}) {
  return createMacNativeComputerUseClientWithAdapters({
    scriptPath,
    tmpDir,
    runNative: runSwiftComputerUse,
    screenshotCapture: captureScreenshot,
    removeFile: removeScreenshot,
  });
}

function createMacNativeComputerUseClientWithAdapters({
  scriptPath,
  tmpDir,
  runNative,
  screenshotCapture,
  removeFile,
} = {}) {
  let lastScreenshotPath = null;
  const root = tmpDir ?? os.tmpdir();
  return {
    async observe() {
      let screenshot = null;
      try {
        screenshot = await screenshotCapture(root);
        const native = await runNative(scriptPath, "observe", {});
        const previousScreenshotPath = lastScreenshotPath;
        lastScreenshotPath = screenshot.path;
        await removeFile(previousScreenshotPath);
        return {
          ...native,
          screenshot,
        };
      } catch (error) {
        await removeFile(screenshot?.path ?? null);
        throw error;
      }
    },
    async act(action) {
      await runNative(scriptPath, action.type, action);
    },
    async cleanup() {
      await removeFile(lastScreenshotPath);
      lastScreenshotPath = null;
    },
  };
}

async function runSwiftComputerUse(scriptPath, command, payload) {
  const encoded = Buffer.from(JSON.stringify(payload ?? {})).toString("base64");
  const { stdout } = await execFileAsync("/usr/bin/swift", [scriptPath, command, encoded], {
    maxBuffer: 1024 * 1024,
  });
  const parsed = JSON.parse(String(stdout || "{}"));
  if (!parsed.ok) {
    throw new Error(parsed.error || `${command} failed`);
  }
  delete parsed.ok;
  return parsed;
}

async function captureScreenshot(tmpDir) {
  const file = path.join(tmpDir, `morpheus-computer-use-${randomUUID()}.png`);
  await execFileAsync("/usr/sbin/screencapture", ["-x", "-t", "png", file], {
    maxBuffer: 1024 * 1024,
  });
  const stat = await fs.stat(file);
  if (stat.size > MAX_SCREENSHOT_BYTES) {
    await fs.rm(file, { force: true });
    throw new Error("Screenshot is too large to return safely");
  }
  const bytes = await fs.readFile(file);
  return {
    path: file,
    mimeType: "image/png",
    byteSize: stat.size,
    dataUrl: `data:image/png;base64,${bytes.toString("base64")}`,
  };
}

async function removeScreenshot(file) {
  if (file) {
    await fs.rm(file, { force: true });
  }
}

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}

module.exports = {
  createComputerUseManager,
  createMacNativeComputerUseClient,
  createMacNativeComputerUseClientWithAdapters,
  classifyComputerUseAction,
  normalizeAction,
  normalizeModifiers,
  targetMismatch,
  shouldAllowComputerUseAction: (action) => classifyComputerUseAction(normalizeAction(action)),
};

const { execFile } = require("node:child_process");
const fsSync = require("node:fs");
const fs = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");
const { randomUUID } = require("node:crypto");
const { promisify } = require("node:util");

const execFileAsync = promisify(execFile);
const MAX_SCREENSHOT_BYTES = 12 * 1024 * 1024;
const MAX_TRACE_ITEMS = 80;
const MAX_POINTER_PATH_ITEMS = 48;
const AGENT_CURSOR_MOVE_SAMPLES = 10;
const AGENT_CURSOR_MOVE_DURATION_MS = 180;
const MAC_NATIVE_RESOURCE_RELATIVE_PATH = path.join(
  "native",
  "computerUseMacNative.swift",
);
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
          options.scriptPath ?? resolveMacNativeComputerUseScriptPath(options),
        tmpDir: options.tmpDir,
    });
    this.clock = options.clock ?? (() => Date.now());
    this.overlayController = options.overlayController ?? options.overlay ?? null;
    this.session = null;
  }

  async cleanup() {
    await this.nativeClient.cleanup?.();
    await this.overlayController?.destroy?.();
    this.markOverlayStopped();
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
    await this.overlayController?.destroy?.();
    this.markOverlayStopped();
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

    if (shouldActivateTargetBeforeAction(normalized, this.session)) {
      try {
        await this.activateTargetForAction(traceItem);
        await this.observeNow("post-activation");
      } catch (error) {
        traceItem.status = "failed";
        traceItem.error = errorMessage(error);
        traceItem.completedAtMs = this.clock();
        this.applyError(error, "target-activation");
        return this.state();
      }
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
      if (normalized.type === "move") {
        await this.moveAgentCursor(normalized, traceItem);
      } else {
        if (normalized.type === "click") {
          await this.moveAgentCursor(normalized, traceItem);
        } else if (normalized.type === "drag") {
          await this.dragAgentCursor(normalized, traceItem);
        }
        const actionResult = await this.nativeClient.act(normalized);
        applyActionEvidence(traceItem, actionResult);
      }
      traceItem.status = "completed";
      traceItem.completedAtMs = this.clock();
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
    if (
      action.type !== "move" &&
      this.session.targetVisibility !== "frontmost"
    ) {
      return {
        kind: "target-mismatch",
        allowed: false,
        reason:
          this.session.targetVisibility === "background"
            ? "Target app is still in the background after target activation; refusing to send side-effect Computer Use actions to the current foreground app."
            : "Target app visibility is unknown after target activation; refusing to send side-effect Computer Use actions to the current foreground app.",
      };
    }
    const mismatch =
      action.type === "move"
        ? null
        : targetMismatch(this.session.target, this.session.observation?.frontmostApp);
    if (mismatch) {
      return {
        kind: "target-mismatch",
        allowed: false,
        reason: mismatch,
      };
    }
    return basePolicy;
  }

  async activateTargetForAction(traceItem) {
    const targetApp = this.session.target?.app;
    if (!targetApp) {
      return;
    }
    const startedAtMs = this.clock();
    const activation = {
      attempted: true,
      status: "running",
      targetApp,
      startedAtMs,
      before: {
        observationSequence: this.session.observation?.sequence ?? null,
        targetVisibility: this.session.targetVisibility ?? "unknown",
        frontmostApp: compactAppIdentity(this.session.frontmostApp),
        targetApp: compactAppIdentity(this.session.targetApp),
      },
    };
    traceItem.activation = activation;
    this.session.status = "activating-target";
    this.session.updatedAtMs = startedAtMs;
    const markActivationFailed = (reason) => {
      activation.status = "failed";
      activation.completedAtMs = this.clock();
      activation.result = {
        activated: false,
        reason,
      };
    };
    const activateTarget = this.nativeClient.activateTarget;
    if (typeof activateTarget !== "function") {
      const message = "Native Computer Use backend does not support target activation";
      markActivationFailed(message);
      throw new Error(message);
    }
    let result;
    try {
      result = await activateTarget.call(this.nativeClient, {
        targetApp,
        observedTargetApp: compactAppIdentity(this.session.targetApp),
        frontmostApp: compactAppIdentity(this.session.frontmostApp),
      });
    } catch (error) {
      markActivationFailed(errorMessage(error));
      throw error;
    }
    activation.status = result?.activated === false ? "failed" : "completed";
    activation.completedAtMs = this.clock();
    activation.result = normalizeActivationResult(result);
    if (result?.activated === false) {
      throw new Error(result.reason || `Could not activate target app ${targetApp}`);
    }
  }

  state() {
    this.ensureSession();
    return snapshotSession(this.session);
  }

  async observeNow(reason) {
    const observation = await this.nativeClient.observe({
      targetApp: this.session?.target?.app ?? null,
    });
    this.applyObservation(observation, reason);
    if (reason !== "post-action") {
      await this.updateOverlay({
        durationMs: 0,
        pathSamples: [],
      });
    }
  }

  ensureSession() {
    if (!this.session) {
      this.session = createEmptySession(this.clock(), {});
    }
  }

  applyObservation(observation, reason) {
    const now = this.clock();
    const previousSequence = this.session.observation?.sequence ?? 0;
    const systemCursor = normalizePoint(observation.systemCursor ?? observation.cursor);
    const frontmostApp = observation.frontmostApp ?? observation.activeApp ?? null;
    const targetApp = resolveObservedTargetApp(this.session.target, observation);
    const targetVisibility = resolveTargetVisibility(
      this.session.target,
      frontmostApp,
      targetApp,
      observation.targetVisibility,
    );
    const limitations = resolveObservationLimitations({
      hasTarget: Boolean(this.session.target.app),
      targetApp,
      targetVisibility,
    });
    this.session.observation = {
      sequence: previousSequence + 1,
      reason,
      observedAtMs: now,
      cursor: systemCursor,
      systemCursor,
      activeApp: frontmostApp,
      frontmostApp,
      targetApp,
      targetVisibility,
      limitations,
      accessibilityTrusted: observation.accessibilityTrusted !== false,
      screenshot: observation.screenshot ?? null,
      error: null,
    };
    if (!this.session.target.app && frontmostApp) {
      this.session.target.app =
        frontmostApp.bundleIdentifier || frontmostApp.name || null;
    }
    this.session.target.window = resolveTargetWindow({
      frontmostApp,
      target: this.session.target,
      targetApp,
      targetVisibility,
    });
    this.session.frontmostApp = frontmostApp;
    this.session.targetApp = targetApp;
    this.session.targetVisibility = targetVisibility;
    this.session.limitations = limitations;
    this.session.systemCursor = systemCursor;
    if (!this.session.agentCursor && systemCursor) {
      this.session.agentCursor = systemCursor;
      this.session.cursor = systemCursor;
      pushPointerPoint(this.session, systemCursor, now, "agent-init");
    } else {
      this.session.cursor = this.session.agentCursor;
    }
    this.session.status = "active";
    this.session.error = null;
    this.session.updatedAtMs = now;
    const currentTrace = this.session.trace.at(-1);
    if (
      reason === "post-activation" &&
      currentTrace?.activation &&
      !currentTrace.activation.reobserved
    ) {
      currentTrace.activation.reobserved = {
        observationSequence: this.session.observation.sequence,
        targetVisibility,
        frontmostApp: compactAppIdentity(frontmostApp),
        targetApp: compactAppIdentity(targetApp),
      };
    }
  }

  async moveAgentCursor(action, traceItem) {
    const now = this.clock();
    const destination = { x: action.x, y: action.y };
    const origin =
      this.session.agentCursor ??
      this.session.systemCursor ??
      this.session.observation?.systemCursor ??
      destination;
    const pathSamples = buildAgentCursorPath(origin, destination, {
      steps: AGENT_CURSOR_MOVE_SAMPLES,
      atMs: now,
      source: action.type,
    });
    this.session.agentCursor = destination;
    this.session.cursor = destination;
    for (const sample of pathSamples) {
      pushPointerPoint(this.session, sample, sample.atMs, sample.source);
    }
    traceItem.agentCursorPath = pathSamples.map(({ x, y, atMs }) => ({
      x,
      y,
      atMs,
    }));
    await this.updateOverlay({
      durationMs: AGENT_CURSOR_MOVE_DURATION_MS,
      pathSamples,
    });
  }

  async dragAgentCursor(action, traceItem) {
    const now = this.clock();
    const origin = { x: action.from.x, y: action.from.y };
    const destination = { x: action.to.x, y: action.to.y };
    const pathSamples = buildAgentCursorPath(origin, destination, {
      steps: AGENT_CURSOR_MOVE_SAMPLES,
      atMs: now,
      source: action.type,
    });
    this.session.agentCursor = destination;
    this.session.cursor = destination;
    for (const sample of pathSamples) {
      pushPointerPoint(this.session, sample, sample.atMs, sample.source);
    }
    traceItem.agentCursorPath = pathSamples.map(({ x, y, atMs }) => ({
      x,
      y,
      atMs,
    }));
    await this.updateOverlay({
      durationMs: AGENT_CURSOR_MOVE_DURATION_MS,
      pathSamples,
    });
  }

  async updateOverlay({ durationMs, pathSamples }) {
    if (!this.overlayController || !this.session?.agentCursor) {
      if (this.session) {
        this.session.overlay = {
          mode: "target-bound",
          visible: false,
          reason: !this.overlayController
            ? "Computer Use overlay controller is unavailable for this session."
            : "Agent cursor overlay is hidden because no agent cursor is available.",
        };
      }
      return;
    }
    const targetVisibility = this.session.targetVisibility ?? "unknown";
    if (targetVisibility === "background" || targetVisibility === "unknown") {
      this.session.overlay = {
        mode: "target-bound",
        visible: false,
        reason:
          targetVisibility === "background"
            ? "Agent cursor is tracking a background target; target-bound overlay is hidden to avoid drawing over the user's current foreground app. After the target becomes frontmost, a subsequent observe or move can show target-bound cursor feedback."
            : "Agent cursor overlay is hidden because target visibility is unknown.",
      };
      await this.overlayController.destroy?.();
      return;
    }
    const targetBounds = targetWindowBounds(this.session.target?.window);
    if (!targetBounds) {
      this.session.overlay = {
        mode: "target-bound",
        visible: false,
        reason:
          "Agent cursor overlay is hidden because target window bounds are unavailable.",
      };
      await this.overlayController.destroy?.();
      return;
    }
    if (!pointInRect(this.session.agentCursor, targetBounds)) {
      this.session.overlay = {
        mode: "target-bound",
        visible: false,
        reason:
          "Agent cursor overlay is hidden because the agent cursor is outside the target window visible bounds.",
      };
      await this.overlayController.destroy?.();
      return;
    }
    const overlayResult = await this.overlayController.update?.({
      agentCursor: this.session.agentCursor,
      durationMs,
      pathSamples: pathSamples.filter((point) => pointInRect(point, targetBounds)),
      pointerPath: this.session.pointerPath,
      status: this.session.status,
      targetBounds,
      targetVisibility,
    });
    if (
      overlayResult &&
      (overlayResult.visible === false || overlayResult.available === false)
    ) {
      this.session.overlay = {
        mode: "target-bound",
        visible: false,
        reason:
          overlayResult.reason ||
          "Computer Use overlay helper is unavailable for this session.",
      };
      return;
    }
    this.session.overlay = { mode: "target-bound", visible: true, reason: null };
  }

  markOverlayStopped() {
    if (this.session) {
      this.session.overlay = {
        mode: "target-bound",
        visible: false,
        reason: "Agent cursor overlay is stopped.",
      };
    }
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
    frontmostApp: null,
    targetApp: null,
    targetVisibility: "unknown",
    systemCursor: null,
    agentCursor: null,
    cursor: null,
    pointerPath: [],
    limitations: [],
    overlay: { mode: "target-bound", visible: false, reason: null },
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
  const modifiers = normalizeModifiers(action.modifiers);
  if (
    (action.type === "type" && DANGEROUS_TEXT_PATTERN.test(action.text ?? "")) ||
    (action.type === "key" &&
      modifiers.includes("cmd") &&
      ["delete", "q", "w"].includes(String(action.key ?? "").toLowerCase()))
  ) {
    return {
      kind: "side-effect-warning",
      allowed: true,
      reason:
        "Potentially destructive or sensitive action is being executed because run --actions is the explicit Computer Use operation boundary.",
    };
  }
  if (["click", "type", "key", "drag"].includes(action.type)) {
    return {
      kind: "side-effect",
      allowed: true,
      reason: "Computer Use side-effect action.",
    };
  }
  return { kind: "low-risk", allowed: true, reason: null };
}

function isSideEffectAction(action) {
  return ["click", "type", "key", "drag"].includes(action?.type);
}

function shouldActivateTargetBeforeAction(action, session) {
  return (
    isSideEffectAction(action) &&
    Boolean(session?.target?.app) &&
    session.targetVisibility !== "frontmost"
  );
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

function resolveObservedTargetApp(target, observation) {
  if (observation.targetApp) {
    return observation.targetApp;
  }
  const frontmostApp = observation.frontmostApp ?? observation.activeApp ?? null;
  if (!target?.app) {
    return frontmostApp;
  }
  return targetMismatch(target, frontmostApp) ? null : frontmostApp;
}

function resolveTargetVisibility(target, frontmostApp, targetApp, nativeVisibility) {
  if (nativeVisibility === "frontmost" || nativeVisibility === "background") {
    return nativeVisibility;
  }
  if (!target?.app) {
    return frontmostApp ? "frontmost" : "unknown";
  }
  if (!targetApp) {
    return "unknown";
  }
  return targetMismatch(target, frontmostApp) ? "background" : "frontmost";
}

function resolveObservationLimitations({ hasTarget, targetApp, targetVisibility }) {
  const limitations = [];
  if (hasTarget && !targetApp) {
    limitations.push({
      code: "target-app-not-observed",
      message: "The target app is not visible to the current macOS observation pass.",
    });
  }
  if (targetVisibility === "background") {
    limitations.push({
      code: "background-observe-metadata-only",
      message:
        "Background target observe is limited to app/window metadata; the screenshot remains the current desktop capture.",
    });
  }
  return limitations;
}

function resolveTargetWindow({ frontmostApp, target, targetApp, targetVisibility }) {
  if (targetApp?.window) {
    return targetApp.window;
  }
  if (!target?.app || targetVisibility === "frontmost") {
    return frontmostApp?.window ?? null;
  }
  return null;
}

function normalizeTargetApp(value) {
  return String(value).trim().toLowerCase();
}

function compactAppIdentity(app) {
  if (!app || typeof app !== "object") {
    return null;
  }
  const result = {};
  for (const key of ["name", "bundleIdentifier", "processIdentifier", "frontmost"]) {
    if (app[key] !== undefined && app[key] !== null) {
      result[key] = app[key];
    }
  }
  if (app.window && typeof app.window === "object") {
    result.window = {};
    for (const key of ["title", "role", "subrole"]) {
      if (app.window[key] !== undefined && app.window[key] !== null) {
        result.window[key] = app.window[key];
      }
    }
  }
  return Object.keys(result).length > 0 ? result : null;
}

function normalizeActivationResult(result) {
  if (!result || typeof result !== "object") {
    return null;
  }
  const normalized = {
    activated: result.activated === true,
  };
  if (typeof result.reason === "string") {
    normalized.reason = result.reason;
  }
  if (Number.isFinite(result.waitedMs)) {
    normalized.waitedMs = result.waitedMs;
  }
  if (result.targetApp) {
    normalized.targetApp = compactAppIdentity(result.targetApp);
  }
  if (result.frontmostApp) {
    normalized.frontmostApp = compactAppIdentity(result.frontmostApp);
  }
  return normalized;
}

function appendTrace(session, item) {
  session.trace.push(item);
  if (session.trace.length > MAX_TRACE_ITEMS) {
    session.trace.splice(0, session.trace.length - MAX_TRACE_ITEMS);
  }
}

function pushPointerPoint(session, point, atMs, source) {
  session.pointerPath.push({ ...point, atMs, source });
  if (session.pointerPath.length > MAX_POINTER_PATH_ITEMS) {
    session.pointerPath.splice(
      0,
      session.pointerPath.length - MAX_POINTER_PATH_ITEMS,
    );
  }
}

function buildAgentCursorPath(origin, destination, options = {}) {
  const steps = Math.max(2, Math.trunc(options.steps ?? AGENT_CURSOR_MOVE_SAMPLES));
  const atMs = options.atMs ?? Date.now();
  const source = options.source ?? "move";
  const pathSamples = [];
  for (let index = 0; index < steps; index += 1) {
    const ratio = index / (steps - 1);
    pathSamples.push({
      x: roundPointCoordinate(origin.x + (destination.x - origin.x) * ratio),
      y: roundPointCoordinate(origin.y + (destination.y - origin.y) * ratio),
      atMs: atMs + Math.round(ratio * AGENT_CURSOR_MOVE_DURATION_MS),
      source,
    });
  }
  return pathSamples;
}

function roundPointCoordinate(value) {
  return Math.round(value * 100) / 100;
}

function applyActionEvidence(traceItem, actionResult) {
  if (!actionResult || typeof actionResult !== "object") {
    return;
  }
  const evidence = { ...(traceItem.evidence ?? {}) };
  if (
    Object.hasOwn(actionResult, "systemCursorRestored") ||
    Object.hasOwn(actionResult, "systemCursorBefore") ||
    Object.hasOwn(actionResult, "systemCursorAfter")
  ) {
    evidence.systemCursorRestored = actionResult.systemCursorRestored === true;
    evidence.systemCursorBefore = normalizePoint(actionResult.systemCursorBefore);
    evidence.systemCursorAfter = normalizePoint(actionResult.systemCursorAfter);
  }
  if (Object.hasOwn(actionResult, "ok")) {
    evidence.ok = actionResult.ok === true;
  }
  if (Number.isFinite(actionResult.characterCount)) {
    evidence.characterCount = actionResult.characterCount;
  }
  if (typeof actionResult.key === "string") {
    evidence.key = actionResult.key;
  }
  if (Array.isArray(actionResult.modifiers)) {
    evidence.modifiers = actionResult.modifiers
      .filter((value) => typeof value === "string")
      .slice(0, 8);
  }
  if (Object.keys(evidence).length > 0) {
    traceItem.evidence = evidence;
  }
}

function targetWindowBounds(window) {
  const position = normalizePoint(window?.position);
  const width = Number(window?.size?.width);
  const height = Number(window?.size?.height);
  if (
    !position ||
    !Number.isFinite(width) ||
    !Number.isFinite(height) ||
    width <= 0 ||
    height <= 0
  ) {
    return null;
  }
  return { x: position.x, y: position.y, width, height };
}

function pointInRect(point, rect) {
  if (!point || !rect) {
    return false;
  }
  return (
    point.x >= rect.x &&
    point.y >= rect.y &&
    point.x <= rect.x + rect.width &&
    point.y <= rect.y + rect.height
  );
}

function createMacNativeComputerUseClient({ scriptPath, tmpDir } = {}) {
  return createMacNativeComputerUseClientWithAdapters({
    scriptPath: scriptPath ?? resolveMacNativeComputerUseScriptPath(),
    tmpDir,
    runNative: runSwiftComputerUse,
    screenshotCapture: captureScreenshot,
    removeFile: removeScreenshot,
  });
}

function resolveMacNativeComputerUseScriptPath(options = {}) {
  const sourceDirectory = options.sourceDirectory ?? __dirname;
  const resourcesPath =
    options.resourcesPath === undefined
      ? currentResourcesPath()
      : options.resourcesPath;
  const fileExists =
    options.fileExists ?? ((targetPath) => fsSync.existsSync(targetPath));
  if (resourcesPath) {
    const resourcePath = path.join(resourcesPath, MAC_NATIVE_RESOURCE_RELATIVE_PATH);
    if (fileExists(resourcePath)) {
      return resourcePath;
    }
  }
  return path.join(sourceDirectory, "computerUseMacNative.swift");
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
    async observe(payload = {}) {
      let screenshot = null;
      try {
        screenshot = await screenshotCapture(root);
        const native = await runNative(scriptPath, "observe", payload);
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
      return runNative(scriptPath, action.type, action);
    },
    async activateTarget(payload) {
      return runNative(scriptPath, "activate", payload);
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

function currentResourcesPath() {
  return typeof process.resourcesPath === "string"
    ? process.resourcesPath
    : null;
}

module.exports = {
  MAC_NATIVE_RESOURCE_RELATIVE_PATH,
  buildAgentCursorPath,
  createComputerUseManager,
  createMacNativeComputerUseClient,
  createMacNativeComputerUseClientWithAdapters,
  classifyComputerUseAction,
  normalizeAction,
  normalizeModifiers,
  resolveMacNativeComputerUseScriptPath,
  targetMismatch,
  shouldAllowComputerUseAction: (action) => classifyComputerUseAction(normalizeAction(action)),
};

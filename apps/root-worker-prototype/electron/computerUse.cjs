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
const DEFAULT_PERCEPTION_LIMIT = 40;
const MAX_PERCEPTION_LIMIT = 80;
const AGENT_CURSOR_MOVE_SAMPLES = 10;
const AGENT_CURSOR_MOVE_DURATION_MS = 180;
const MAX_WAIT_ACTION_MS = 10_000;
const MAC_NATIVE_RESOURCE_RELATIVE_PATH = path.join(
  "native",
  "computerUseMacNative.swift",
);
const MAC_NATIVE_HELPER_EXECUTABLE_FILE = "morpheus-computer-use-native";
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
    this.perception = {
      enabled: options.includePerception !== false,
      limit: boundedPerceptionLimit(options.perceptionLimit),
    };
    this.safety = normalizeSafetyOptions(options.safety);
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
    let policy = this.applyRuntimeSafetyPolicy(classifyComputerUseAction(normalized));
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
      audit: createActionAudit(policy, this.session, this.safety, "initial"),
    };
    appendTrace(this.session, traceItem);
    if (!policy.allowed) {
      completeActionAudit(traceItem, this.session, "blocked");
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
      completeActionAudit(traceItem, this.session, "failed");
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
        completeActionAudit(traceItem, this.session, "failed");
        this.applyError(error, "target-activation");
        return this.state();
      }
    }

    policy = this.policyForAction(normalized);
    traceItem.policy = policy;
    traceItem.observationSequence = this.session.observation?.sequence ?? null;
    traceItem.audit = createActionAudit(policy, this.session, this.safety, "pre-action");
    if (!policy.allowed) {
      traceItem.status = "blocked";
      traceItem.error = policy.reason;
      traceItem.completedAtMs = this.clock();
      completeActionAudit(traceItem, this.session, "blocked");
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
      if (normalized.type === "findText") {
        applyActionEvidence(traceItem, findTextInObservation(normalized, this.session.observation));
      } else if (normalized.type === "move") {
        await this.moveAgentCursor(normalized, traceItem);
      } else if (normalized.type === "wait") {
        await delay(normalized.ms);
        applyActionEvidence(traceItem, {
          ok: true,
          method: "timer",
          waitMs: normalized.ms,
        });
      } else if (normalized.type === "clickText") {
        const match = findTextInObservation(normalized, this.session.observation);
        applyActionEvidence(traceItem, match);
        if (!match.ok || match.matchStatus !== "unique") {
          throw new Error(match.reason || `Could not uniquely match visible text: ${normalized.text}`);
        }
        await this.moveAgentCursor({ type: "click", ...match.point }, traceItem);
        const actionResult = await this.nativeClient.act({
          type: "click",
          ...match.point,
        });
        applyActionEvidence(traceItem, {
          ...actionResult,
          compiledAction: { type: "click", ...match.point },
        });
      } else if (normalized.type === "setText") {
        const match = findTextInObservation(
          { ...normalized, requireWritable: true },
          this.session.observation,
        );
        applyActionEvidence(traceItem, {
          ...match,
          targetVisibility: this.session.targetVisibility ?? "unknown",
          characterCount: normalized.text.length,
        });
        if (!match.ok || match.matchStatus !== "unique") {
          throw new Error(match.reason || `Could not uniquely match writable text: ${normalized.query}`);
        }
        const actionResult = await this.nativeClient.act({
          type: "setText",
          targetApp: this.session.target?.app,
          query: normalized.query,
          text: normalized.text,
        });
        applyActionEvidence(traceItem, actionResult);
      } else {
        if (["click", "doubleClick", "rightClick", "scroll"].includes(normalized.type)) {
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
      completeActionAudit(traceItem, this.session, "completed");
    } catch (error) {
      traceItem.status = "failed";
      traceItem.error = errorMessage(error);
      traceItem.completedAtMs = this.clock();
      completeActionAudit(traceItem, this.session, "failed");
      this.applyError(error, "action");
    } finally {
      this.session.pendingAction = null;
    }
    return this.state();
  }

  policyForAction(action) {
    const basePolicy = this.applyRuntimeSafetyPolicy(classifyComputerUseAction(action));
    if (!basePolicy.allowed) {
      return basePolicy;
    }
    if (this.session.observation?.accessibilityTrusted === false) {
      return {
        ...basePolicy,
        kind: "needs-permission",
        allowed: false,
        reason: "Accessibility permission is required before controlling the desktop.",
      };
    }
    if (!this.session.observation) {
      return {
        ...basePolicy,
        kind: "needs-observation",
        allowed: false,
        reason: "Observe the desktop before running Computer Use actions.",
      };
    }
    if (
      isForegroundRequiredSideEffectAction(action) &&
      this.session.targetVisibility !== "frontmost"
    ) {
      return {
        ...basePolicy,
        kind: "target-mismatch",
        allowed: false,
        reason:
          this.session.targetVisibility === "background"
            ? "Target app is still in the background after target activation; refusing to send side-effect Computer Use actions to the current foreground app."
            : "Target app visibility is unknown after target activation; refusing to send side-effect Computer Use actions to the current foreground app.",
      };
    }
    const mismatch =
      action.type === "move" || !isForegroundRequiredSideEffectAction(action)
        ? null
        : targetMismatch(this.session.target, this.session.observation?.frontmostApp);
    if (mismatch) {
      return {
        ...basePolicy,
        kind: "target-mismatch",
        allowed: false,
        reason: mismatch,
      };
    }
    return basePolicy;
  }

  applyRuntimeSafetyPolicy(policy) {
    return applyRuntimeSafetyPolicy(policy, this.safety);
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
      includePerception: this.perception.enabled,
      perceptionLimit: this.perception.limit,
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
    const perception = normalizeObservationPerception({
      observation,
      frontmostApp,
      targetApp,
      targetVisibility,
      screenshot: observation.screenshot ?? null,
      limit: this.perception.limit,
      enabled: this.perception.enabled,
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
      perception,
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

function normalizeSafetyOptions(options = {}) {
  return {
    operationBoundary:
      typeof options.operationBoundary === "string" && options.operationBoundary.length > 0
        ? options.operationBoundary
        : "manager-session",
    confirmRisk: normalizeConfirmedRisk(options.confirmRisk ?? options.confirmRiskLevel),
    planOnly: options.planOnly === true || options.dryRun === true,
  };
}

function normalizeConfirmedRisk(value) {
  if (value === true) {
    return "high";
  }
  if (typeof value !== "string") {
    return null;
  }
  const normalized = value.toLowerCase();
  if (["high", "all"].includes(normalized)) {
    return normalized;
  }
  return null;
}

function createActionAudit(policy, session, safety, phase) {
  return {
    version: "computer-use-audit-v1",
    phase,
    operationBoundary: safety.operationBoundary,
    planOnly: safety.planOnly,
    riskLevel: policy.riskLevel ?? "unknown",
    riskCategories: Array.isArray(policy.riskCategories)
      ? policy.riskCategories.slice(0, 8)
      : [],
    requiresConfirmation: policy.requiresConfirmation === true,
    confirmationSatisfied: policy.confirmationSatisfied === true,
    confirmation: {
      required: policy.requiresConfirmation === true,
      satisfied: policy.confirmationSatisfied === true,
      confirmedRisk: safety.confirmRisk ?? null,
    },
    before: observationAuditEvidence(session),
    after: null,
    completion: null,
  };
}

function completeActionAudit(traceItem, session, phase) {
  if (!traceItem.audit) {
    return;
  }
  traceItem.audit.phase = phase;
  traceItem.audit.after = observationAuditEvidence(session);
  traceItem.audit.completion = {
    status: traceItem.status,
    completedAtMs: traceItem.completedAtMs ?? null,
    actionEvidence: actionEvidenceSummary(traceItem),
    afterObservationSequence: session.observation?.sequence ?? null,
  };
}

function observationAuditEvidence(session) {
  const observation = session?.observation ?? null;
  return {
    observationSequence: observation?.sequence ?? null,
    targetVisibility: session?.targetVisibility ?? observation?.targetVisibility ?? "unknown",
    frontmostApp: compactAppIdentity(session?.frontmostApp ?? observation?.frontmostApp),
    targetApp: compactAppIdentity(session?.targetApp ?? observation?.targetApp),
    accessibilityTrusted: observation?.accessibilityTrusted ?? null,
  };
}

function actionEvidenceSummary(traceItem) {
  const evidence = traceItem.evidence ?? {};
  if (traceItem.action?.type === "move") {
    return { source: "agent-cursor", pathSamples: traceItem.agentCursorPath?.length ?? 0 };
  }
  if (traceItem.action?.type === "findText") {
    return { source: "perception", matchStatus: evidence.matchStatus ?? null };
  }
  if (traceItem.action?.type === "wait") {
    return { source: "timer", waitMs: evidence.waitMs ?? null };
  }
  if (traceItem.action?.type === "clickText") {
    const source =
      evidence.matchStatus === "unique" && evidence.compiledAction
        ? "native-bridge"
        : "perception";
    return {
      source,
      ok: evidence.ok === true,
      matchStatus: evidence.matchStatus ?? null,
      compiledAction: evidence.compiledAction ?? null,
    };
  }
  if (traceItem.action?.type === "setText") {
    return {
      source:
        evidence.method === "accessibility-set-value"
          ? "native-bridge"
          : "perception",
      ok: evidence.ok === true,
      method: evidence.method ?? null,
      matchStatus: evidence.matchStatus ?? null,
      targetVisibility: evidence.targetVisibility ?? null,
      characterCount: evidence.characterCount ?? null,
    };
  }
  return {
    source: isSideEffectAction(traceItem.action) ? "native-bridge" : "manager",
    ok: evidence.ok ?? null,
    method: evidence.method ?? null,
  };
}

function snapshotSession(session) {
  return JSON.parse(JSON.stringify(session));
}

function classifyComputerUseAction(action) {
  if (action.type === "observe") {
    return withRisk({
      kind: "read-only",
      allowed: true,
      reason: null,
    }, "low", ["read-only"]);
  }
  const modifiers = normalizeModifiers(action.modifiers);
  const riskyText =
    action.type === "type"
      ? action.text
      : action.type === "clickText"
        ? action.text
        : action.type === "setText"
          ? action.text
        : "";
  if (
    ((action.type === "type" || action.type === "clickText" || action.type === "setText") &&
      DANGEROUS_TEXT_PATTERN.test(riskyText ?? "")) ||
    ((action.type === "key" || action.type === "hotkey") &&
      modifiers.includes("cmd") &&
      ["delete", "q", "w"].includes(String(action.key ?? "").toLowerCase()))
  ) {
    return withRisk({
      kind: "side-effect-warning",
      allowed: true,
      reason:
        "Potentially destructive or sensitive Computer Use action requires explicit risk confirmation before native input is sent.",
    }, "high", riskyActionCategories(action));
  }
  if (
    [
      "click",
      "clickText",
      "doubleClick",
      "rightClick",
      "scroll",
      "type",
      "key",
      "hotkey",
      "drag",
      "setText",
    ].includes(action.type)
  ) {
    return withRisk({
      kind: "side-effect",
      allowed: true,
      reason: "Computer Use side-effect action.",
    }, "medium", ["native-side-effect"]);
  }
  return withRisk({ kind: "low-risk", allowed: true, reason: null }, "low", ["agent-evidence"]);
}

function withRisk(policy, riskLevel, riskCategories) {
  return {
    ...policy,
    riskLevel,
    riskCategories,
    requiresConfirmation: false,
    confirmationSatisfied: false,
    operationBoundary: null,
  };
}

function riskyActionCategories(action) {
  const categories = ["native-side-effect", "sensitive-or-destructive"];
  if (action.type === "type" || action.type === "clickText" || action.type === "setText") {
    categories.push("text-intent");
  }
  if (action.type === "key" || action.type === "hotkey") {
    categories.push("destructive-shortcut");
  }
  return categories;
}

function applyRuntimeSafetyPolicy(policy, safety) {
  const operationBoundary = safety.operationBoundary;
  const requiresConfirmation =
    policy.riskLevel === "high" && policy.allowed === true;
  const confirmationSatisfied =
    requiresConfirmation && riskLevelAllowed(policy.riskLevel, safety.confirmRisk);
  if (safety.planOnly && isRiskySideEffectPolicy(policy)) {
    return {
      ...policy,
      kind: "plan-only",
      allowed: false,
      reason:
        "Computer Use plan-only mode blocked this real desktop side-effect before native input.",
      requiresConfirmation,
      confirmationSatisfied,
      operationBoundary,
    };
  }
  if (requiresConfirmation && !confirmationSatisfied) {
    return {
      ...policy,
      kind: "requires-confirmation",
      allowed: false,
      reason:
        "High-risk Computer Use action requires --confirm-risk high before native input is sent.",
      requiresConfirmation: true,
      confirmationSatisfied: false,
      operationBoundary,
    };
  }
  return {
    ...policy,
    requiresConfirmation,
    confirmationSatisfied,
    operationBoundary,
  };
}

function isRiskySideEffectPolicy(policy) {
  return policy.riskLevel === "medium" || policy.riskLevel === "high";
}

function riskLevelAllowed(riskLevel, confirmedRisk) {
  const order = { low: 0, medium: 1, high: 2 };
  if (!confirmedRisk) {
    return false;
  }
  if (confirmedRisk === "all") {
    return true;
  }
  return order[confirmedRisk] >= order[riskLevel];
}

function isSideEffectAction(action) {
  return [
    "click",
    "clickText",
    "doubleClick",
    "rightClick",
    "scroll",
    "type",
    "key",
    "hotkey",
    "drag",
    "setText",
  ].includes(action?.type);
}

function shouldActivateTargetBeforeAction(action, session) {
  return (
    isForegroundRequiredSideEffectAction(action) &&
    Boolean(session?.target?.app) &&
    session.targetVisibility !== "frontmost"
  );
}

function isForegroundRequiredSideEffectAction(action) {
  return isSideEffectAction(action) && action?.type !== "setText";
}

function normalizeAction(action) {
  if (!action || typeof action !== "object") {
    throw new Error("Computer Use action is required");
  }
  switch (action.type) {
    case "move":
    case "click":
    case "doubleClick":
    case "rightClick":
      return { type: action.type, ...requirePoint(action) };
    case "scroll":
      return {
        type: "scroll",
        ...requirePoint(action),
        deltaX: finiteNumberOrDefault(action.deltaX, 0, "deltaX"),
        deltaY: finiteNumberOrDefault(action.deltaY, 0, "deltaY"),
      };
    case "findText":
      return {
        type: "findText",
        text: requireTextQuery(action.text),
        maxMatches: boundedMaxMatches(action.maxMatches),
      };
    case "clickText":
      return {
        type: "clickText",
        text: requireTextQuery(action.text),
        maxMatches: boundedMaxMatches(action.maxMatches),
      };
    case "setText":
      return {
        type: "setText",
        query: requireTextQuery(action.query ?? action.target ?? action.textQuery),
        text: String(action.text ?? ""),
        maxMatches: boundedMaxMatches(action.maxMatches),
      };
    case "type":
      return { type: "type", text: String(action.text ?? "") };
    case "key":
      return {
        type: "key",
        key: String(action.key ?? ""),
        modifiers: normalizeModifiers(action.modifiers),
      };
    case "hotkey":
      return {
        type: "hotkey",
        key: String(action.key ?? ""),
        modifiers: normalizeModifiers(action.modifiers),
      };
    case "drag":
      return {
        type: "drag",
        from: requirePoint(action.from ?? {}),
        to: requirePoint(action.to ?? {}),
      };
    case "wait":
    case "pause":
      return {
        type: "wait",
        ms: boundedWaitMs(action.ms ?? action.waitMs ?? action.durationMs),
      };
    default:
      throw new Error(`Unsupported Computer Use action: ${String(action.type)}`);
  }
}

function finiteNumberOrDefault(value, defaultValue, name) {
  if (value === undefined || value === null || value === "") {
    return defaultValue;
  }
  const parsed = Number(value);
  if (!Number.isFinite(parsed)) {
    throw new Error(`Action requires finite ${name}`);
  }
  return parsed;
}

function requireTextQuery(value) {
  const text = String(value ?? "").trim();
  if (!text) {
    throw new Error("Text targeting action requires non-empty text");
  }
  return text;
}

function boundedMaxMatches(value) {
  if (value === undefined || value === null || value === "") {
    return 8;
  }
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < 1 || parsed > 20) {
    throw new Error("Text targeting maxMatches must be between 1 and 20");
  }
  return parsed;
}

function boundedPerceptionLimit(value) {
  if (value === undefined || value === null || value === "") {
    return DEFAULT_PERCEPTION_LIMIT;
  }
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < 0 || parsed > MAX_PERCEPTION_LIMIT) {
    throw new Error(`Perception limit must be between 0 and ${MAX_PERCEPTION_LIMIT}`);
  }
  return parsed;
}

function boundedWaitMs(value) {
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < 0 || parsed > MAX_WAIT_ACTION_MS) {
    throw new Error(`Wait action requires ms between 0 and ${MAX_WAIT_ACTION_MS}`);
  }
  return parsed;
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
  if (typeof actionResult.method === "string") {
    evidence.method = actionResult.method;
  }
  if (typeof actionResult.button === "string") {
    evidence.button = actionResult.button;
  }
  if (Number.isFinite(actionResult.clickCount)) {
    evidence.clickCount = actionResult.clickCount;
  }
  if (Number.isFinite(actionResult.deltaX)) {
    evidence.deltaX = actionResult.deltaX;
  }
  if (Number.isFinite(actionResult.deltaY)) {
    evidence.deltaY = actionResult.deltaY;
  }
  if (Number.isFinite(actionResult.waitMs)) {
    evidence.waitMs = actionResult.waitMs;
  }
  if (typeof actionResult.matchStatus === "string") {
    evidence.matchStatus = actionResult.matchStatus;
  }
  if (typeof actionResult.query === "string") {
    evidence.query = actionResult.query;
  }
  if (typeof actionResult.reason === "string") {
    evidence.reason = actionResult.reason;
  }
  if (Number.isSafeInteger(actionResult.matchCount)) {
    evidence.matchCount = actionResult.matchCount;
  }
  if (Number.isSafeInteger(actionResult.characterCount)) {
    evidence.characterCount = actionResult.characterCount;
  }
  if (Array.isArray(actionResult.candidates)) {
    evidence.candidates = actionResult.candidates.slice(0, 20);
  }
  if (actionResult.matchedElement && typeof actionResult.matchedElement === "object") {
    evidence.matchedElement = normalizeAccessibilityElement(actionResult.matchedElement);
  }
  if (typeof actionResult.targetVisibility === "string") {
    evidence.targetVisibility = actionResult.targetVisibility;
  }
  if (actionResult.point) {
    evidence.point = normalizePoint(actionResult.point);
  }
  if (actionResult.compiledAction) {
    evidence.compiledAction = actionResult.compiledAction;
  }
  if (Object.hasOwn(actionResult, "pasteboardRestored")) {
    evidence.pasteboardRestored = actionResult.pasteboardRestored === true;
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

async function delay(ms) {
  if (!Number.isSafeInteger(ms) || ms <= 0) {
    return;
  }
  await new Promise((resolve) => setTimeout(resolve, ms));
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

function normalizeObservationPerception({
  observation,
  frontmostApp,
  targetApp,
  targetVisibility,
  screenshot,
  limit,
  enabled,
}) {
  const limitations = [];
  const targetWindow = targetWindowBounds(targetApp?.window);
  const frontmostWindow = targetWindowBounds(frontmostApp?.window);
  const targetIsFrontmost = targetVisibility === "frontmost";
  const targetIsReadable = targetIsFrontmost || targetVisibility === "background";
  if (!enabled) {
    limitations.push({
      code: "perception-disabled",
      message: "Perception extraction was disabled for this observation.",
    });
  }
  if (!targetIsReadable) {
    limitations.push({
      code: "target-unavailable",
      message: "Target window crop and AX element extraction require a resolved target app.",
    });
  }
  if (observation.accessibilityTrusted === false) {
    limitations.push({
      code: "accessibility-permission-required",
      message: "Accessibility permission is required for AX element extraction.",
    });
  }

  const nativePerception =
    observation.perception && typeof observation.perception === "object"
      ? observation.perception
      : {};
  const nativeLimitations = Array.isArray(nativePerception.limitations)
    ? nativePerception.limitations
    : [];
  for (const item of nativeLimitations) {
    const normalized = normalizeLimitation(item);
    if (normalized) {
      limitations.push(normalized);
    }
  }

  const windowBounds = targetIsFrontmost
    ? targetWindow ?? frontmostWindow
    : targetVisibility === "background"
      ? targetWindow
      : null;
  const windowCrop =
    enabled && targetIsReadable && windowBounds
      ? {
          source: "target-window",
          coordinateSpace: "screen",
          bounds: windowBounds,
          screenshot: normalizeCropScreenshot(
            nativePerception.windowCrop?.screenshot,
            targetIsFrontmost ? screenshot : null,
          ),
        }
      : null;
  if (enabled && targetIsReadable && !windowBounds) {
    limitations.push({
      code: "target-window-bounds-unavailable",
      message: "Target window bounds were not available for crop metadata.",
    });
  }
  if (
    enabled &&
    targetVisibility === "background" &&
    windowBounds &&
    !nativePerception.windowCrop?.screenshot
  ) {
    limitations.push({
      code: "background-window-capture-unavailable",
      message: "Background target window bounds were available, but a native window screenshot was not available.",
    });
  }

  const elements = enabled && targetIsReadable
    ? normalizeAccessibilityElements(nativePerception.accessibilityElements, limit)
    : [];
  if (enabled && targetIsReadable && Array.isArray(nativePerception.accessibilityElements)) {
    const total = nativePerception.accessibilityElements.length;
    if (total > elements.length) {
      limitations.push({
        code: "accessibility-elements-truncated",
        message: `Accessibility element output was truncated to ${elements.length} candidates.`,
      });
    }
  }

  return {
    enabled: enabled === true,
    source: "macos-accessibility",
    limit,
    coordinateSpace: "screen",
    windowCrop,
    accessibilityElements: elements,
    limitations: dedupeLimitations(limitations),
  };
}

function normalizeCropScreenshot(cropScreenshot, fullScreenshot) {
  if (cropScreenshot && typeof cropScreenshot === "object") {
    return {
      mimeType: cropScreenshot.mimeType ?? "image/png",
      byteSize: Number.isFinite(cropScreenshot.byteSize) ? cropScreenshot.byteSize : null,
      dataUrl: typeof cropScreenshot.dataUrl === "string" ? cropScreenshot.dataUrl : undefined,
      path: typeof cropScreenshot.path === "string" ? cropScreenshot.path : undefined,
    };
  }
  if (!fullScreenshot) {
    return null;
  }
  return {
    mimeType: fullScreenshot.mimeType ?? "image/png",
    byteSize: null,
    derivedFrom: "full-screenshot",
    dataUrlOmitted: true,
  };
}

function normalizeAccessibilityElements(elements, limit) {
  if (!Array.isArray(elements) || limit <= 0) {
    return [];
  }
  return elements
    .map(normalizeAccessibilityElement)
    .filter(Boolean)
    .slice(0, limit);
}

function normalizeAccessibilityElement(element) {
  if (!element || typeof element !== "object") {
    return null;
  }
  const bounds = normalizeRect(element.bounds);
  const center = normalizePoint(element.center) ?? rectCenter(bounds);
  if (!bounds || !center) {
    return null;
  }
  const result = {
    role: boundedString(element.role, 64),
    subrole: boundedString(element.subrole, 64),
    title: boundedString(element.title, 160),
    value: boundedString(element.value, 160),
    description: boundedString(element.description, 160),
    writable: element.writable === true,
    bounds,
    center,
    confidence: Number.isFinite(element.confidence) ? element.confidence : 0.8,
    source: boundedString(element.source, 64) ?? "macos-accessibility",
  };
  return Object.fromEntries(
    Object.entries(result).filter(([, value]) => value !== null && value !== undefined),
  );
}

function normalizeRect(value) {
  if (!value || typeof value !== "object") {
    return null;
  }
  const x = Number(value.x);
  const y = Number(value.y);
  const width = Number(value.width);
  const height = Number(value.height);
  if (
    !Number.isFinite(x) ||
    !Number.isFinite(y) ||
    !Number.isFinite(width) ||
    !Number.isFinite(height) ||
    width <= 0 ||
    height <= 0
  ) {
    return null;
  }
  return { x, y, width, height };
}

function rectCenter(rect) {
  if (!rect) {
    return null;
  }
  return {
    x: roundPointCoordinate(rect.x + rect.width / 2),
    y: roundPointCoordinate(rect.y + rect.height / 2),
  };
}

function boundedString(value, maxLength) {
  if (typeof value !== "string") {
    return null;
  }
  const trimmed = value.replace(/\s+/g, " ").trim();
  if (!trimmed) {
    return null;
  }
  return trimmed.length > maxLength ? `${trimmed.slice(0, maxLength - 1)}…` : trimmed;
}

function normalizeLimitation(value) {
  if (!value || typeof value !== "object") {
    return null;
  }
  const code = boundedString(value.code, 80);
  const message = boundedString(value.message, 240);
  return code && message ? { code, message } : null;
}

function dedupeLimitations(limitations) {
  const seen = new Set();
  const result = [];
  for (const item of limitations) {
    const normalized = normalizeLimitation(item);
    if (!normalized || seen.has(normalized.code)) {
      continue;
    }
    seen.add(normalized.code);
    result.push(normalized);
  }
  return result;
}

function findTextInObservation(action, observation) {
  const incompleteReason = perceptionCompletenessReason(observation?.perception);
  if ((action.type === "clickText" || action.type === "setText") && incompleteReason) {
    return {
      ok: false,
      method: "accessibility-text-match",
      matchStatus: "incomplete",
      query: action.query ?? action.text,
      matchCount: 0,
      candidates: [],
      reason: incompleteReason,
    };
  }
  const rawQuery = action.query ?? action.text;
  const query = rawQuery.toLowerCase();
  const elements = observation?.perception?.accessibilityElements ?? [];
  const allCandidates = elements
    .map((element, index) => {
      const haystack = [element.title, element.value, element.description]
        .filter((value) => typeof value === "string" && value.length > 0)
        .join(" ")
        .toLowerCase();
      if (!haystack.includes(query)) {
        return null;
      }
      return {
        index,
        role: element.role ?? null,
        title: element.title ?? null,
        value: element.value ?? null,
        description: element.description ?? null,
        writable: element.writable === true,
        bounds: element.bounds,
        center: element.center,
        confidence: element.confidence ?? null,
        source: element.source ?? null,
      };
    })
    .filter(Boolean);
  const writableCandidates = action.requireWritable
    ? allCandidates.filter((candidate) => candidate.writable === true)
    : allCandidates;
  const consideredCandidates = action.requireWritable ? writableCandidates : allCandidates;
  const candidates = consideredCandidates.slice(0, action.maxMatches);
  if (candidates.length === 0) {
    return {
      ok: false,
      method: "accessibility-text-match",
      matchStatus: "none",
      query: rawQuery,
      matchCount: consideredCandidates.length,
      candidates: [],
      reason: action.requireWritable && allCandidates.length > 0
        ? `No writable accessibility element matched text: ${rawQuery}`
        : `No visible accessibility element matched text: ${rawQuery}`,
    };
  }
  if (consideredCandidates.length > 1) {
    return {
      ok: false,
      method: "accessibility-text-match",
      matchStatus: "multiple",
      query: rawQuery,
      matchCount: consideredCandidates.length,
      candidates,
      reason: `Multiple visible accessibility elements matched text: ${rawQuery}`,
    };
  }
  const point = normalizePoint(candidates[0].center);
  return {
    ok: Boolean(point),
    method: "accessibility-text-match",
    matchStatus: point ? "unique" : "missing-bounds",
    query: rawQuery,
    matchCount: consideredCandidates.length,
    candidates,
    point,
    reason: point ? null : `Matched text but the candidate has no usable center: ${rawQuery}`,
  };
}

function perceptionCompletenessReason(perception) {
  if (!perception || perception.enabled !== true) {
    return "Text targeting requires enabled target perception.";
  }
  const blockingCodes = new Set([
    "accessibility-elements-truncated",
    "accessibility-traversal-truncated",
    "accessibility-permission-required",
    "perception-disabled",
    "target-not-frontmost",
    "target-unavailable",
    "target-window-unavailable",
  ]);
  const limitation = (perception.limitations ?? []).find((item) =>
    blockingCodes.has(item?.code),
  );
  return limitation?.message ?? null;
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
  const backend = resolveMacNativeComputerUseBackend({ scriptPath });
  return createMacNativeComputerUseClientWithAdapters({
    scriptPath: backend.scriptPath,
    executablePath: backend.executablePath,
    tmpDir,
    runNative: backend.executablePath
      ? runNativeComputerUseExecutable
      : runSwiftComputerUse,
    screenshotCapture: backend.executablePath
      ? captureScreenshotWithNativeExecutable
      : captureScreenshotWithScreencapture,
    removeFile: removeScreenshot,
  });
}

function resolveMacNativeComputerUseBackend(options = {}) {
  const envExecutablePath =
    options.executablePath ??
    process.env.MORPHEUS_COMPUTER_USE_NATIVE_HELPER_EXECUTABLE ??
    null;
  const executablePath = isUsableMacNativeExecutablePath(envExecutablePath, options)
    ? envExecutablePath
    : resolveMacNativeComputerUseExecutablePath(options);
  if (executablePath) {
    return {
      mode: "packaged-native-helper-executable",
      executablePath,
      scriptPath: null,
    };
  }
  return {
    mode: "delegated-swift-script",
    executablePath: null,
    scriptPath: options.scriptPath ?? resolveMacNativeComputerUseScriptPath(options),
  };
}

function resolveMacNativeComputerUseExecutablePath(options = {}) {
  const helperBundlePath =
    options.helperBundlePath ?? process.env.MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_PATH;
  const isExecutable =
    options.isExecutable ?? ((targetPath) => isExecutableFile(targetPath));
  if (!helperBundlePath) {
    return null;
  }
  const candidate = path.join(
    helperBundlePath,
    "Contents",
    "MacOS",
    MAC_NATIVE_HELPER_EXECUTABLE_FILE,
  );
  return isExecutable(candidate) ? candidate : null;
}

function isUsableMacNativeExecutablePath(targetPath, options = {}) {
  if (!targetPath) {
    return false;
  }
  const isExecutable =
    options.isExecutable ??
    options.fileExists ??
    ((candidate) => isExecutableFile(candidate));
  return isExecutable(targetPath);
}

function isExecutableFile(targetPath) {
  try {
    const stat = fsSync.statSync(targetPath);
    if (!stat.isFile()) {
      return false;
    }
    fsSync.accessSync(targetPath, fsSync.constants.X_OK);
    return true;
  } catch {
    return false;
  }
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
  executablePath,
  tmpDir,
  runNative,
  screenshotCapture,
  removeFile,
} = {}) {
  let lastScreenshotPath = null;
  let lastCropScreenshotPath = null;
  const root = tmpDir ?? os.tmpdir();
  return {
    async observe(payload = {}) {
      let screenshot = null;
      let cropScreenshot = null;
      try {
        screenshot = await screenshotCapture(root, null, executablePath);
        const native = await runNative(
          executablePath ?? scriptPath,
          "observe",
          payload,
        );
        const cropBounds =
          native.targetVisibility === "frontmost"
            ? targetWindowBounds(native.targetApp?.window ?? native.frontmostApp?.window)
            : null;
        const windowId = Number(native.targetApp?.window?.windowId);
        const captureTarget =
          native.targetVisibility === "background" && Number.isSafeInteger(windowId)
            ? { windowId }
            : cropBounds;
        if (payload.includePerception !== false && captureTarget) {
          try {
            cropScreenshot = await screenshotCapture(
              root,
              captureTarget,
              executablePath,
            );
            native.perception = {
              ...(native.perception ?? {}),
              windowCrop: {
                ...(native.perception?.windowCrop ?? {}),
                screenshot: cropScreenshot,
              },
            };
          } catch (error) {
            if (native.targetVisibility !== "background") {
              throw error;
            }
          }
        }
        const previousScreenshotPath = lastScreenshotPath;
        const previousCropScreenshotPath = lastCropScreenshotPath;
        lastScreenshotPath = screenshot.path;
        lastCropScreenshotPath = cropScreenshot?.path ?? null;
        if (previousScreenshotPath) {
          await removeFile(previousScreenshotPath);
        }
        if (previousCropScreenshotPath) {
          await removeFile(previousCropScreenshotPath);
        }
        return {
          ...native,
          screenshot,
        };
      } catch (error) {
        if (screenshot?.path) {
          await removeFile(screenshot.path);
        }
        if (cropScreenshot?.path) {
          await removeFile(cropScreenshot.path);
        }
        throw error;
      }
    },
    async act(action) {
      return runNative(executablePath ?? scriptPath, action.type, action);
    },
    async activateTarget(payload) {
      return runNative(executablePath ?? scriptPath, "activate", payload);
    },
    async cleanup() {
      if (lastScreenshotPath) {
        await removeFile(lastScreenshotPath);
      }
      if (lastCropScreenshotPath) {
        await removeFile(lastCropScreenshotPath);
      }
      lastScreenshotPath = null;
      lastCropScreenshotPath = null;
    },
  };
}

async function runNativeComputerUseExecutable(executablePath, command, payload) {
  const encoded = Buffer.from(JSON.stringify(payload ?? {})).toString("base64");
  let stdout;
  try {
    ({ stdout } = await execFileAsync(executablePath, [command, encoded], {
      maxBuffer: 1024 * 1024,
    }));
  } catch (error) {
    const nativeError = parseNativeError(error.stdout);
    throw new Error(nativeError || errorMessage(error));
  }
  const parsed = JSON.parse(String(stdout || "{}"));
  if (!parsed.ok) {
    throw new Error(parsed.error || `${command} failed`);
  }
  delete parsed.ok;
  return parsed;
}

async function runSwiftComputerUse(scriptPath, command, payload) {
  const encoded = Buffer.from(JSON.stringify(payload ?? {})).toString("base64");
  let stdout;
  try {
    ({ stdout } = await execFileAsync("/usr/bin/swift", [scriptPath, command, encoded], {
      maxBuffer: 1024 * 1024,
    }));
  } catch (error) {
    const nativeError = parseNativeError(error.stdout);
    throw new Error(nativeError || errorMessage(error));
  }
  const parsed = JSON.parse(String(stdout || "{}"));
  if (!parsed.ok) {
    throw new Error(parsed.error || `${command} failed`);
  }
  delete parsed.ok;
  return parsed;
}

function parseNativeError(stdout) {
  try {
    const parsed = JSON.parse(String(stdout || "{}"));
    return typeof parsed.error === "string" && parsed.error.length > 0
      ? parsed.error
      : null;
  } catch {
    return null;
  }
}

async function captureScreenshotWithNativeExecutable(
  tmpDir,
  bounds = null,
  executablePath,
) {
  const result = await runNativeComputerUseExecutable(
    executablePath,
    "screenshot",
    { tmpDir, bounds },
  );
  return readScreenshotFile(result.screenshot?.path);
}

async function captureScreenshotWithScreencapture(tmpDir, bounds = null) {
  const file = path.join(tmpDir, `morpheus-computer-use-${randomUUID()}.png`);
  const args = ["-x", "-t", "png"];
  const windowId = Number(bounds?.windowId);
  const rect = Number.isSafeInteger(windowId) ? null : normalizeRect(bounds);
  if (Number.isSafeInteger(windowId)) {
    args.push("-l", String(windowId));
  }
  if (rect) {
    args.push(
      "-R",
      `${Math.round(rect.x)},${Math.round(rect.y)},${Math.round(rect.width)},${Math.round(rect.height)}`,
    );
  }
  args.push(file);
  await execFileAsync("/usr/sbin/screencapture", args, {
    maxBuffer: 1024 * 1024,
  });
  return readScreenshotFile(file);
}

async function readScreenshotFile(file) {
  if (!file) {
    throw new Error("Screenshot backend did not return a file path");
  }
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
  MAC_NATIVE_HELPER_EXECUTABLE_FILE,
  buildAgentCursorPath,
  createComputerUseManager,
  createMacNativeComputerUseClient,
  createMacNativeComputerUseClientWithAdapters,
  classifyComputerUseAction,
  normalizeAction,
  normalizeModifiers,
  resolveMacNativeComputerUseBackend,
  resolveMacNativeComputerUseExecutablePath,
  resolveMacNativeComputerUseScriptPath,
  targetMismatch,
  shouldAllowComputerUseAction: (action) => classifyComputerUseAction(normalizeAction(action)),
};

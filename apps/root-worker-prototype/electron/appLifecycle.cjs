"use strict";

const FULL_ACTIVATION_EXIT_CODE = 75;

function createAppRelaunchAdapter({
  app,
  beforeExit,
  setTimeout: scheduleExit = setTimeout,
  exitDelayMs = 50,
  logger = console,
} = {}) {
  let requested = false;
  return {
    requestRelaunch(reason = null) {
      if (requested) {
        return { ok: true, relaunching: true, alreadyRequested: true };
      }
      if (!app || typeof app.relaunch !== "function" || typeof app.exit !== "function") {
        return {
          ok: false,
          relaunching: false,
          reason: "Application relaunch is unavailable in this environment",
        };
      }
      try {
        app.relaunch();
      } catch (error) {
        return {
          ok: false,
          relaunching: false,
          reason: error instanceof Error ? error.message : String(error),
        };
      }
      requested = true;
      scheduleExit(async () => {
        try {
          await beforeExit?.(reason);
        } catch (error) {
          logger?.error?.(
            "[prototype] app relaunch cleanup failed",
            JSON.stringify({
              reason: error instanceof Error ? error.message : String(error),
            }),
          );
        }
        app.exit(0);
      }, exitDelayMs);
      return {
        ok: true,
        relaunching: true,
        alreadyRequested: false,
        reason,
      };
    },
  };
}

function isClientRelaunchNotification(notification) {
  if (!notification || typeof notification !== "object") {
    return false;
  }
  if (notification.method === "client/relaunch/requested") {
    return true;
  }
  if (notification.method !== "client/lifecycle/actionRequested") {
    return false;
  }
  const action = notification.params?.action ?? notification.params?.kind;
  return action === "relaunch" || action === "restart";
}

function createClientRelaunchNotificationHandler({
  installedArtifactUpdate,
  fullRelaunch,
} = {}) {
  let inFlight = null;
  return function handleClientRelaunchNotification(notification) {
    const reason = notification?.params?.reason ?? notification?.method ?? null;
    const requestId = normalizeClientRelaunchRequestId(
      notification?.params?.requestId,
    );
    if (notification?.params?.mode === "hot") {
      return Promise.resolve({
        ok: false,
        unsupported: true,
        legacyModeRejected: true,
        inPlace: false,
        relaunching: false,
        reloaded: false,
        updated: false,
        ...requestIdFields(requestId),
        reason:
          "Legacy hot runtime refresh is unsupported; Runtime Capsules always restart the complete Runtime",
      });
    }
    if (
      notification?.params?.mode !== undefined &&
      notification.params.mode !== "full"
    ) {
      return Promise.resolve({
        ok: false,
        unsupported: true,
        inPlace: false,
        relaunching: false,
        reloaded: false,
        updated: false,
        ...requestIdFields(requestId),
        reason: `Unsupported legacy runtime restart mode: ${String(notification.params.mode)}`,
      });
    }
    if (inFlight) {
      return inFlight.then((result) => ({
        ...result,
        alreadyRequested: true,
        ...requestIdFields(requestId ?? result?.requestId),
      }));
    }
    inFlight = runClientRelaunchNotification({
      fullRelaunch,
      installedArtifactUpdate,
      reason,
      requestId,
    }).finally(() => {
      inFlight = null;
    });
    return inFlight;
  };
}

async function runClientRelaunchNotification({
  fullRelaunch,
  installedArtifactUpdate,
  reason,
  requestId,
}) {
  if (
    installedArtifactUpdate &&
    typeof installedArtifactUpdate.requestUpdateAndRelaunch === "function"
  ) {
    const result = await installedArtifactUpdate.requestUpdateAndRelaunch(
      reason,
      requestId,
    );
    if (!result?.unsupported || result?.disabled) {
      return result;
    }
  }
  const relaunch =
    fullRelaunch && typeof fullRelaunch.requestRelaunch === "function"
      ? await fullRelaunch.requestRelaunch(reason)
      : {
          ok: false,
          relaunching: false,
          reason: "Application relaunch adapter is unavailable",
        };
  return {
    ok: Boolean(relaunch.ok),
    inPlace: false,
    relaunching: Boolean(relaunch.relaunching),
    reloaded: false,
    updated: false,
    ...requestIdFields(requestId),
    relaunch,
    reason: relaunch.reason ?? reason,
  };
}

async function observeClientRelaunchResult(
  resultPromise,
  { broadcastStatus, logger = console, reason = null, requestId = null } = {},
) {
  try {
    const result = await resultPromise;
    broadcastStatus?.({
      lifecycle: {
        type: "clientRelaunch",
        phase: result?.ok ? "completed" : "failed",
        ...requestIdFields(result?.requestId ?? requestId),
        reason: result?.reason ?? reason,
      },
      relaunch:
        result && typeof result === "object"
          ? result.relaunch ?? result
          : result ?? null,
    });
    return result;
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    logger?.error?.(
      "[prototype] client relaunch notification failed",
      JSON.stringify({ reason: message }),
    );
    broadcastStatus?.({
      lifecycle: {
        type: "clientRelaunch",
        phase: "failed",
        ...requestIdFields(requestId),
        reason: message,
      },
    });
    return {
      ok: false,
      inPlace: false,
      relaunching: false,
      reloaded: false,
      updated: false,
      ...requestIdFields(requestId),
      reason: message,
    };
  }
}

function createInstalledArtifactUpdateLifecycleAdapter({
  appExit,
  appServerStop,
  cleanupPreparedArtifact,
  resolvePlan,
  runtimeLauncher,
  updateArtifacts,
  broadcastStatus,
  logger = console,
} = {}) {
  let inFlight = null;
  return {
    requestUpdateAndRelaunch(reason = null, requestId = null) {
      if (inFlight) {
        return inFlight.then((result) => ({
          ...result,
          alreadyRequested: true,
          ...requestIdFields(requestId ?? result?.requestId),
        }));
      }
      if (typeof resolvePlan !== "function") {
        return Promise.resolve({ ok: false, unsupported: true });
      }
      inFlight = resolveAndRunInstalledArtifactUpdate({
        appExit,
        appServerStop,
        cleanupPreparedArtifact,
        resolvePlan,
        runtimeLauncher,
        updateArtifacts,
        broadcastStatus,
        logger,
        reason,
        requestId,
      }).finally(() => {
        inFlight = null;
      });
      return inFlight;
    },
  };
}

async function resolveAndRunInstalledArtifactUpdate({
  appExit,
  appServerStop,
  cleanupPreparedArtifact,
  resolvePlan,
  runtimeLauncher,
  updateArtifacts,
  broadcastStatus,
  logger,
  reason,
  requestId,
}) {
  let plan;
  try {
    plan = await resolvePlan();
  } catch (error) {
    return installedUpdateFailure(error, {
      broadcastStatus,
      logger,
      phase: "planning",
      reason,
      requestId,
    });
  }
  if (!plan) {
    return { ok: false, unsupported: true };
  }
  if (plan.disabled) {
    return {
      ok: false,
      unsupported: true,
      disabled: true,
      inPlace: false,
      relaunching: false,
      reloaded: false,
      updated: false,
      ...requestIdFields(requestId),
      reason:
        plan.reason ??
        "Runtime Capsule candidate production is unavailable",
    };
  }
  return runInstalledArtifactUpdate({
    appExit,
    appServerStop,
    cleanupPreparedArtifact,
    plan,
    runtimeLauncher,
    updateArtifacts,
    broadcastStatus,
    logger,
    reason,
    requestId,
  });
}

async function runInstalledArtifactUpdate({
  appExit,
  appServerStop,
  cleanupPreparedArtifact,
  plan,
  runtimeLauncher,
  updateArtifacts,
  broadcastStatus,
  logger,
  reason,
  requestId,
}) {
  if (typeof updateArtifacts !== "function") {
    return {
      ok: false,
      unsupported: false,
      relaunching: false,
      updated: false,
      ...requestIdFields(requestId),
      reason: "Runtime Capsule producer is unavailable",
    };
  }
  broadcastStatus?.({
    lifecycle: {
      type: "installedArtifactUpdate",
      phase: "preparing",
      ...requestIdFields(requestId),
      reason,
    },
  });
  let update = null;
  let prepared = null;
  try {
    update = await updateArtifacts(plan);
    if (!update?.ok || !update.activationId || !update.releaseId) {
      throw new Error(
        update?.reason ?? "Runtime Capsule production did not complete",
      );
    }
    if (!runtimeLauncher?.supported) {
      throw new Error("Runtime Capsule launcher is unavailable");
    }
    prepared = await runtimeLauncher.prepareActivation({
      activationId: update.activationId,
      manifest: update.manifest,
      reason,
      releaseId: update.releaseId,
    });
    if (
      prepared?.activationId !== update.activationId ||
      prepared?.releaseId !== update.releaseId
    ) {
      throw new Error(
        "Runtime Capsule prepare result does not match the produced candidate",
      );
    }
    if (prepared.disposition === "terminal_failed") {
      const outcome = prepared.control?.activation?.receipt?.outcome;
      throw new Error(
        `Runtime Capsule activation is terminally unavailable${outcome ? ` (${outcome})` : ""}`,
      );
    }
    if (prepared.disposition === "already_committed") {
      if (update?.incomingRoot) {
        await cleanupPreparedCandidate(
          cleanupPreparedArtifact,
          update.incomingRoot,
          logger,
        );
      }
      return {
        ok: true,
        activationId: update.activationId,
        releaseId: update.releaseId,
        inPlace: false,
        relaunching: false,
        reloaded: false,
        updated: true,
        alreadyCommitted: true,
        prepared,
        ...requestIdFields(requestId),
        reason,
      };
    }
    if (
      prepared.disposition !== "prepared" &&
      prepared.disposition !== "already_prepared"
    ) {
      throw new Error(
        `Runtime Capsule prepare returned unsupported disposition: ${String(prepared.disposition)}`,
      );
    }
    broadcastStatus?.({
      lifecycle: {
        type: "installedArtifactUpdate",
        phase: "prepared",
        activationId: update.activationId,
        releaseId: update.releaseId,
        ...requestIdFields(requestId),
        reason,
      },
    });
    const backendStop = await stopUpdatedAppServer(appServerStop, reason);
    if (!backendStop.ok) {
      throw await cancelPreparedActivation(
        backendStop.reason ??
          "App-server stop failed before Runtime Capsule activation",
        runtimeLauncher,
        update.activationId,
        { backendStop },
      );
    }
    if (typeof appExit !== "function") {
      throw await cancelPreparedActivation(
        "Application exit is unavailable after Runtime Capsule preparation",
        runtimeLauncher,
        update.activationId,
        { backendStop },
      );
    }
    appExit(FULL_ACTIVATION_EXIT_CODE);
    const relaunch = {
      ok: true,
      relaunching: true,
      supervised: true,
      exitCode: FULL_ACTIVATION_EXIT_CODE,
    };
    broadcastStatus?.({
      lifecycle: {
        type: "installedArtifactUpdate",
        phase: "relaunching",
        activationId: update.activationId,
        releaseId: update.releaseId,
        ...requestIdFields(requestId),
        reason,
      },
      relaunch,
    });
    return {
      ok: true,
      activationId: update.activationId,
      releaseId: update.releaseId,
      inPlace: false,
      relaunching: true,
      reloaded: false,
      updated: true,
      backendStop,
      prepared,
      relaunch,
      ...requestIdFields(requestId),
      reason,
    };
  } catch (error) {
    if (update?.incomingRoot) {
      await cleanupPreparedCandidate(
        cleanupPreparedArtifact,
        update.incomingRoot,
        logger,
      );
    }
    return installedUpdateFailure(error, {
      broadcastStatus,
      logger,
      phase: prepared ? "prepared" : "preparing",
      reason,
      requestId,
      update,
    });
  }
}

async function cancelPreparedActivation(
  message,
  runtimeLauncher,
  activationId,
  details,
) {
  let cancellation;
  try {
    cancellation = await runtimeLauncher.cancelActivation(
      activationId,
      message,
    );
  } catch (error) {
    cancellation = {
      ok: false,
      reason: error instanceof Error ? error.message : String(error),
    };
  }
  const suffix =
    cancellation?.ok === false
      ? `; activation cancellation also failed: ${cancellation.reason ?? "unknown failure"}`
      : "";
  return Object.assign(new Error(`${message}${suffix}`), {
    ...details,
    cancellation,
    updated: false,
  });
}

async function cleanupPreparedCandidate(cleanup, preparedRoot, logger) {
  if (!preparedRoot || typeof cleanup !== "function") {
    return;
  }
  try {
    await cleanup(preparedRoot);
  } catch (error) {
    logger?.warn?.(
      "[prototype] Runtime Capsule cleanup failed",
      JSON.stringify({
        reason: error instanceof Error ? error.message : String(error),
      }),
    );
  }
}

async function stopUpdatedAppServer(appServerStop, reason) {
  if (!appServerStop || typeof appServerStop.requestStop !== "function") {
    return {
      ok: false,
      reason: "App-server stop adapter is unavailable",
    };
  }
  try {
    const result = await appServerStop.requestStop(
      reason ?? "Runtime Capsule activation",
    );
    return result?.ok === false
      ? result
      : { ...(result ?? {}), ok: true };
  } catch (error) {
    return {
      ok: false,
      reason: error instanceof Error ? error.message : String(error),
    };
  }
}

function installedUpdateFailure(
  error,
  { broadcastStatus, logger, phase, reason, requestId, update } = {},
) {
  const message = error instanceof Error ? error.message : String(error);
  logger?.error?.(
    "[prototype] Runtime Capsule update failed",
    JSON.stringify({ phase, reason: message }),
  );
  broadcastStatus?.({
    lifecycle: {
      type: "installedArtifactUpdate",
      phase: "failed",
      activationId: update?.activationId ?? null,
      releaseId: update?.releaseId ?? null,
      ...requestIdFields(requestId),
      reason: message,
    },
  });
  return {
    ok: false,
    activationId: update?.activationId ?? null,
    releaseId: update?.releaseId ?? null,
    inPlace: false,
    partial: Boolean(error?.partial),
    relaunching: false,
    reloaded: false,
    updated: Boolean(error?.updated),
    backendStop: error?.backendStop,
    cancellation: error?.cancellation,
    ...requestIdFields(requestId),
    reason: message,
  };
}

function normalizeClientRelaunchRequestId(requestId) {
  return typeof requestId === "string" && requestId.trim()
    ? requestId.trim()
    : null;
}

function requestIdFields(requestId) {
  return requestId ? { requestId } : {};
}

function createRendererReloadLifecycleAdapter({
  fullRelaunch,
  reloadWindows,
  broadcastStatus,
  logger = console,
} = {}) {
  let inFlight = null;
  return {
    requestReload(reason = null) {
      if (inFlight) {
        return inFlight.then((result) => ({
          ...result,
          alreadyRequested: true,
        }));
      }
      inFlight = runRendererReload({
        fullRelaunch,
        reloadWindows,
        broadcastStatus,
        logger,
        reason,
      }).finally(() => {
        inFlight = null;
      });
      return inFlight;
    },
    requestHotReload() {
      return Promise.resolve({
        ok: false,
        unsupported: true,
        inPlace: false,
        relaunching: false,
        reloaded: false,
        reason: "Legacy hot runtime refresh is unsupported",
      });
    },
  };
}

async function runRendererReload({
  fullRelaunch,
  reloadWindows,
  broadcastStatus,
  logger,
  reason,
}) {
  if (typeof reloadWindows !== "function") {
    return requestFullRelaunchFallback(fullRelaunch, reason, {
      broadcastStatus,
      logger,
      reason: "Renderer reload is unavailable in this environment",
    });
  }
  broadcastStatus?.({
    lifecycle: { type: "rendererReload", phase: "reloading", reason },
  });
  try {
    const reload = await reloadWindows({ reason });
    broadcastStatus?.({
      lifecycle: { type: "rendererReload", phase: "reloaded", reason },
    });
    return {
      ok: true,
      inPlace: true,
      relaunching: false,
      reloaded: true,
      alreadyRequested: false,
      windowsReloaded: reload?.windowsReloaded ?? null,
      reason,
    };
  } catch (error) {
    return requestFullRelaunchFallback(fullRelaunch, reason, {
      broadcastStatus,
      logger,
      reason: error instanceof Error ? error.message : String(error),
    });
  }
}

function requestFullRelaunchFallback(
  fullRelaunch,
  reason,
  { reason: fallbackReason, broadcastStatus, logger } = {},
) {
  logger?.warn?.(
    "[prototype] renderer reload unavailable; falling back to full relaunch",
    JSON.stringify({ reason: fallbackReason }),
  );
  const fallback =
    fullRelaunch && typeof fullRelaunch.requestRelaunch === "function"
      ? fullRelaunch.requestRelaunch(reason)
      : {
          ok: false,
          relaunching: false,
          reason: "Application relaunch fallback is unavailable",
        };
  const result = {
    ok: fallback.ok,
    inPlace: false,
    relaunching: Boolean(fallback.relaunching),
    reloaded: false,
    fallback,
    reason: fallback.reason ?? fallbackReason ?? reason,
  };
  broadcastStatus?.({
    lifecycle: {
      type: "rendererReload",
      phase: fallback.ok ? "fullRelaunchFallback" : "failed",
      reason: result.reason,
    },
    relaunch: fallback,
  });
  return result;
}

module.exports = {
  createAppRelaunchAdapter,
  createClientRelaunchNotificationHandler,
  createInstalledArtifactUpdateLifecycleAdapter,
  createRendererReloadLifecycleAdapter,
  isClientRelaunchNotification,
  observeClientRelaunchResult,
};

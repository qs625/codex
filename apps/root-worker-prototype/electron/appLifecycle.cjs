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
      if (!app || typeof app.relaunch !== "function") {
        return {
          ok: false,
          relaunching: false,
          reason: "Application relaunch is unavailable in this environment",
        };
      }
      if (typeof app.exit !== "function") {
        return {
          ok: false,
          relaunching: false,
          reason: "Application exit is unavailable in this environment",
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
        if (typeof beforeExit === "function") {
          try {
            await beforeExit(reason);
          } catch (error) {
            logger?.error?.(
              "[prototype] app relaunch cleanup failed",
              JSON.stringify({
                reason: error instanceof Error ? error.message : String(error),
              }),
            );
          }
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

function createRendererReloadLifecycleAdapter({
  fullRelaunch,
  reloadWindows,
  broadcastStatus,
  logger = console,
} = {}) {
  let inFlight = null;
  let inFlightMode = null;

  function requestReloadWithMode({
    allowFullRelaunchFallback,
    publicMode,
    reason,
    semanticMode,
  }) {
    if (inFlight) {
      if (inFlightMode.semanticMode !== semanticMode) {
        return Promise.resolve(
          restartModeConflictResult({
            requestedMode: publicMode,
            executingMode: inFlightMode.publicMode,
            requestedModeLabel: semanticMode,
            executingModeLabel: inFlightMode.semanticMode,
          }),
        );
      }
      const executingMode = inFlightMode.publicMode;
      return inFlight.then((result) =>
        coalescedRestartResult(result, {
          requestedMode: publicMode,
          executingMode,
        }),
      );
    }

    inFlightMode = { publicMode, semanticMode };
    inFlight = runRendererReload({
      fullRelaunch,
      reloadWindows,
      broadcastStatus,
      logger,
      reason,
      allowFullRelaunchFallback,
      mode: publicMode,
    }).finally(() => {
      inFlight = null;
      inFlightMode = null;
    });
    return inFlight;
  }

  return {
    requestReload(reason = null) {
      return requestReloadWithMode({
        allowFullRelaunchFallback: true,
        publicMode: null,
        reason,
        semanticMode: "generic reload",
      });
    },
    requestHotReload(reason = null) {
      return requestReloadWithMode({
        allowFullRelaunchFallback: false,
        publicMode: "hot",
        reason,
        semanticMode: "hot",
      });
    },
  };
}

function coalescedRestartResult(result, { requestedMode, executingMode }) {
  if (result?.busy || result?.conflict) {
    return {
      ...result,
      alreadyRequested: true,
    };
  }
  return {
    ...result,
    alreadyRequested: true,
    requestedMode,
    executingMode,
  };
}

function restartModeConflictResult({
  requestedMode,
  executingMode,
  requestedModeLabel = requestedMode,
  executingModeLabel = executingMode,
}) {
  return {
    ok: false,
    busy: true,
    conflict: true,
    inPlace: false,
    relaunching: false,
    reloaded: false,
    updated: false,
    mode: null,
    requestedMode,
    executingMode,
    reason: `Runtime refresh mode conflict: requested ${requestedModeLabel} while ${executingModeLabel} is in progress`,
  };
}

function normalizeClientRelaunchMode(mode) {
  if (mode === "hot" || mode === "full") {
    return mode;
  }
  return null;
}

function normalizeClientRelaunchRequestId(requestId) {
  return typeof requestId === "string" && requestId.trim()
    ? requestId.trim()
    : null;
}

function createClientRelaunchNotificationHandler({
  rendererReload,
  installedArtifactUpdate,
  fullRelaunch,
} = {}) {
  let inFlight = null;
  let inFlightMode = null;

  return async function handleClientRelaunchNotification(notification) {
    const reason = notification?.params?.reason ?? notification?.method ?? null;
    const mode = normalizeClientRelaunchMode(notification?.params?.mode);
    const requestId = normalizeClientRelaunchRequestId(
      notification?.params?.requestId,
    );
    if (!mode) {
      return {
        ok: false,
        inPlace: false,
        relaunching: false,
        reloaded: false,
        updated: false,
        mode: null,
        ...requestIdFields(requestId),
        reason: `Invalid client relaunch mode: ${String(notification?.params?.mode ?? "missing")}`,
      };
    }
    if (inFlight) {
      if (inFlightMode !== mode) {
        return restartModeConflictResult({
          requestedMode: mode,
          executingMode: inFlightMode,
        });
      }
      const executingMode = inFlightMode;
      return inFlight.then((result) =>
        coalescedRestartResult(result, {
          requestedMode: mode,
          executingMode,
        }),
      );
    }

    inFlightMode = mode;
    inFlight = runClientRelaunchNotification({
      fullRelaunch,
      installedArtifactUpdate,
      mode,
      reason,
      requestId,
      rendererReload,
    }).finally(() => {
      inFlight = null;
      inFlightMode = null;
    });
    return inFlight;
  };
}

async function runClientRelaunchNotification({
  fullRelaunch,
  installedArtifactUpdate,
  mode,
  reason,
  requestId,
  rendererReload,
}) {
  if (
    installedArtifactUpdate &&
    typeof installedArtifactUpdate.requestUpdateAndRelaunch === "function"
  ) {
    const updateResult =
      await installedArtifactUpdate.requestUpdateAndRelaunch(
        reason,
        mode,
        requestId,
      );
    if (!updateResult.unsupported) {
      return updateResult;
    }
  }
  if (mode === "full") {
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
      mode,
      ...requestIdFields(requestId),
      relaunch,
      reason: relaunch.reason ?? reason,
    };
  }
  if (!rendererReload || typeof rendererReload.requestReload !== "function") {
    return {
      ok: false,
      inPlace: false,
      relaunching: false,
      reloaded: false,
      reason: "Renderer reload adapter is unavailable",
      ...requestIdFields(requestId),
    };
  }
  if (typeof rendererReload.requestHotReload === "function") {
    const result = await rendererReload.requestHotReload(reason);
    return {
      ...(result && typeof result === "object" ? result : {}),
      ...requestIdFields(requestId),
    };
  }
  const result = await rendererReload.requestReload(reason);
  return {
    ...(result && typeof result === "object" ? result : {}),
    ...requestIdFields(requestId),
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
        mode: result?.mode ?? null,
        ...requestIdFields(result?.requestId ?? requestId),
        reason: result?.reason ?? reason,
      },
      relaunch:
        result && typeof result === "object"
          ? { ...(result.relaunch ?? result), mode: result.mode ?? null }
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
        mode: null,
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
  appServerRestart,
  appServerStop,
  fullRelaunch,
  reloadWindows,
  resolvePlan,
  updateArtifacts,
  broadcastStatus,
  logger = console,
} = {}) {
  let inFlight = null;
  let inFlightMode = null;

  return {
    requestUpdateAndRelaunch(reason = null, mode = null, requestId = null) {
      const normalizedMode = normalizeClientRelaunchMode(mode);
      if (!normalizedMode) {
        return Promise.resolve({
          ok: false,
          unsupported: false,
          relaunching: false,
          updated: false,
          mode: null,
          ...requestIdFields(requestId),
          reason: `Invalid installed artifact refresh mode: ${String(mode ?? "missing")}`,
        });
      }
      if (inFlight) {
        if (inFlightMode !== normalizedMode) {
          return Promise.resolve(
            restartModeConflictResult({
              requestedMode: normalizedMode,
              executingMode: inFlightMode,
            }),
          );
        }
        const executingMode = inFlightMode;
        return inFlight.then((result) =>
          coalescedRestartResult(result, {
            requestedMode: normalizedMode,
            executingMode,
          }),
        );
      }
      if (!resolvePlan || typeof resolvePlan !== "function") {
        return Promise.resolve({ ok: false, unsupported: true });
      }

      inFlightMode = normalizedMode;
      inFlight = resolveAndRunInstalledArtifactUpdate({
        appServerRestart,
        appServerStop,
        fullRelaunch,
        reloadWindows,
        updateArtifacts,
        broadcastStatus,
        logger,
        resolvePlan,
        reason,
        mode: normalizedMode,
        requestId,
      }).finally(() => {
        inFlight = null;
        inFlightMode = null;
      });
      return inFlight;
    },
  };
}

async function resolveAndRunInstalledArtifactUpdate({
  appServerRestart,
  appServerStop,
  fullRelaunch,
  reloadWindows,
  updateArtifacts,
  broadcastStatus,
  logger,
  resolvePlan,
  reason,
  mode,
  requestId,
}) {
  let plan;
  try {
    plan = await resolvePlan();
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    logger?.error?.(
      "[prototype] installed artifact update planning failed",
      JSON.stringify({ reason: message }),
    );
    broadcastStatus?.({
      lifecycle: {
        type: "installedArtifactUpdate",
        phase: "failed",
        mode,
        ...requestIdFields(requestId),
        reason: message,
      },
    });
    return {
      ok: false,
      unsupported: false,
      inPlace: false,
      partial: false,
      relaunching: false,
      reloaded: false,
      updated: false,
      mode,
      ...requestIdFields(requestId),
      reason: message,
    };
  }
  if (!plan) {
    return { ok: false, unsupported: true };
  }
  return runInstalledArtifactUpdate({
    appServerRestart,
    appServerStop,
    fullRelaunch,
    reloadWindows,
    updateArtifacts,
    broadcastStatus,
    logger,
    plan,
    reason,
    mode,
    requestId,
  });
}

async function runInstalledArtifactUpdate({
  appServerRestart,
  appServerStop,
  fullRelaunch,
  reloadWindows,
  updateArtifacts,
  broadcastStatus,
  logger,
  plan,
  reason,
  mode,
  requestId,
}) {
  if (!updateArtifacts || typeof updateArtifacts !== "function") {
    return {
      ok: false,
      unsupported: false,
      relaunching: false,
      updated: false,
      mode,
      ...requestIdFields(requestId),
      reason: "Installed artifact updater is unavailable",
    };
  }

  broadcastStatus?.({
    lifecycle: {
      type: "installedArtifactUpdate",
      phase: "building",
      mode,
      ...requestIdFields(requestId),
      reason,
    },
  });

  try {
    const update = await updateArtifacts(plan);
    if (!update?.ok) {
      throw new Error(
        update?.reason ?? "Installed artifact update did not complete",
      );
    }
    broadcastStatus?.({
      lifecycle: {
        type: "installedArtifactUpdate",
        phase: "updated",
        mode,
        ...requestIdFields(requestId),
        reason,
      },
    });
    if (mode === "full") {
      return await relaunchUpdatedApp({
        appServerStop,
        broadcastStatus,
        fullRelaunch,
        mode,
        reason,
        requestId,
        update,
      });
    }
    const backendRestart = await restartUpdatedAppServer(appServerRestart, reason);
    if (!backendRestart.ok) {
      throw partialInstalledUpdateError(
        backendRestart.reason ?? "App-server restart failed after update",
        { backendRestart, updated: Boolean(update.updated) },
      );
    }
    const reload = await reloadUpdatedRenderer(reloadWindows, {
      broadcastStatus,
      mode,
      reason,
      requestId,
    });
    if (!reload.ok) {
      throw partialInstalledUpdateError(
        reload.reason ?? "Renderer reload failed after update",
        {
          backendRestart,
          reload,
          updated: Boolean(update.updated),
        },
      );
    }
    broadcastStatus?.({
      lifecycle: {
        type: "installedArtifactUpdate",
        phase: "reloaded",
        mode,
        ...requestIdFields(requestId),
        reason,
      },
      reload,
    });
    return {
      ok: true,
      inPlace: true,
      relaunching: false,
      reloaded: true,
      updated: Boolean(update.updated),
      backendRestart,
      mode,
      ...requestIdFields(requestId),
      mainProcessUpdate: "pendingAppRelaunch",
      preloadUpdate: "pendingWindowRecreateOrAppRelaunch",
      reason,
      reload,
    };
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    logger?.error?.(
      "[prototype] installed artifact update failed",
      JSON.stringify({ reason: message }),
    );
    broadcastStatus?.({
      lifecycle: {
        type: "installedArtifactUpdate",
        phase: "failed",
        mode,
        ...requestIdFields(requestId),
        reason: message,
      },
    });
    const failure = {
      ok: false,
      inPlace: false,
      partial: Boolean(error?.partial),
      relaunching: false,
      reloaded: false,
      updated: Boolean(error?.updated),
      backendRestart: error?.backendRestart,
      reload: error?.reload,
      mode,
      ...requestIdFields(requestId),
      reason: message,
    };
    if (error?.backendStop) {
      failure.backendStop = error.backendStop;
    }
    if (error?.relaunch) {
      failure.relaunch = error.relaunch;
    }
    return failure;
  }
}

async function relaunchUpdatedApp({
  appServerStop,
  broadcastStatus,
  fullRelaunch,
  mode,
  reason,
  requestId,
  update,
}) {
  broadcastStatus?.({
    lifecycle: {
      type: "installedArtifactUpdate",
      phase: "relaunching",
      mode,
      ...requestIdFields(requestId),
      reason,
    },
  });
  const backendStop = await stopUpdatedAppServer(appServerStop, reason);
  if (!backendStop.ok) {
    throw partialInstalledUpdateError(
      backendStop.reason ?? "App-server stop failed before app relaunch",
      { backendStop, updated: Boolean(update.updated) },
    );
  }
  const relaunch =
    fullRelaunch && typeof fullRelaunch.requestRelaunch === "function"
      ? await fullRelaunch.requestRelaunch(reason)
      : {
          ok: false,
          relaunching: false,
          reason: "Application relaunch adapter is unavailable after shell update.",
        };
  if (!relaunch.ok) {
    throw partialInstalledUpdateError(
      relaunch.reason ?? "Application relaunch failed after shell update",
      { backendStop, relaunch, updated: Boolean(update.updated) },
    );
  }
  broadcastStatus?.({
    lifecycle: {
      type: "installedArtifactUpdate",
      phase: "relaunching",
      mode,
      ...requestIdFields(requestId),
      reason,
    },
    relaunch,
  });
  return {
    ok: true,
    inPlace: false,
    relaunching: Boolean(relaunch.relaunching),
    reloaded: false,
    updated: Boolean(update.updated),
    backendStop,
    mode,
    ...requestIdFields(requestId),
    mainProcessUpdate: "requiresAppRelaunch",
    preloadUpdate: "requiresAppRelaunch",
    reason,
    relaunch,
  };
}

async function stopUpdatedAppServer(appServerStop, reason) {
  if (!appServerStop || typeof appServerStop.requestStop !== "function") {
    return {
      ok: false,
      stopped: false,
      reason:
        "App-server stop adapter is unavailable before installed app relaunch.",
    };
  }
  const result = await appServerStop.requestStop(reason);
  return {
    ok: Boolean(result?.ok),
    stopped: Boolean(result?.stopped ?? result?.ok),
    reason: result?.reason ?? reason,
  };
}

async function restartUpdatedAppServer(appServerRestart, reason) {
  if (!appServerRestart || typeof appServerRestart.requestRestart !== "function") {
    return {
      ok: false,
      restarted: false,
      reason:
        "App-server restart adapter is unavailable after installed artifact update.",
    };
  }
  const result = await appServerRestart.requestRestart(reason);
  return {
    ok: Boolean(result?.ok),
    pending: false,
    restarted: Boolean(result?.restarted ?? result?.ok),
    pid: result?.pid ?? null,
    reason: result?.reason ?? reason,
  };
}

async function reloadUpdatedRenderer(
  reloadWindows,
  { broadcastStatus, mode, reason, requestId },
) {
  if (typeof reloadWindows !== "function") {
    return {
      ok: false,
      inPlace: true,
      relaunching: false,
      reloaded: false,
      mode,
      ...requestIdFields(requestId),
      reason: "Renderer reload is unavailable after installed artifact update",
    };
  }
  broadcastStatus?.({
    lifecycle: {
      type: "installedArtifactUpdate",
      phase: "reloading",
      mode,
      ...requestIdFields(requestId),
      reason,
    },
  });
  try {
    const reload = await reloadWindows({ reason });
    return {
      ok: true,
      inPlace: true,
      relaunching: false,
      reloaded: true,
      windowsReloaded: reload?.windowsReloaded ?? null,
      mode,
      ...requestIdFields(requestId),
      reason,
    };
  } catch (error) {
    return {
      ok: false,
      inPlace: true,
      relaunching: false,
      reloaded: false,
      mode,
      ...requestIdFields(requestId),
      reason: error instanceof Error ? error.message : String(error),
    };
  }
}

function partialInstalledUpdateError(message, details = {}) {
  const error = new Error(message);
  error.partial = true;
  Object.assign(error, details);
  return error;
}

function requestIdFields(requestId) {
  return requestId ? { requestId } : {};
}

async function runRendererReload({
  fullRelaunch,
  reloadWindows,
  broadcastStatus,
  logger,
  reason,
  allowFullRelaunchFallback = true,
  mode = null,
}) {
  if (typeof reloadWindows !== "function") {
    if (!allowFullRelaunchFallback) {
      return {
        ok: false,
        inPlace: true,
        relaunching: false,
        reloaded: false,
        mode,
        reason: "Renderer reload is unavailable in this environment",
      };
    }
    return requestFullRelaunchFallback(fullRelaunch, reason, {
      reason: "Renderer reload is unavailable in this environment",
      broadcastStatus,
      logger,
      mode,
    });
  }

  broadcastStatus?.({
    lifecycle: {
      type: "rendererReload",
      phase: "reloading",
      mode,
      reason,
    },
  });

  try {
    const reload = await reloadWindows({ reason });
    broadcastStatus?.({
      lifecycle: {
        type: "rendererReload",
        phase: "reloaded",
        mode,
        reason,
      },
    });
    return {
      ok: true,
      inPlace: true,
      relaunching: false,
      reloaded: true,
      alreadyRequested: false,
      windowsReloaded: reload?.windowsReloaded ?? null,
      mode,
      reason,
    };
  } catch (error) {
    if (!allowFullRelaunchFallback) {
      return {
        ok: false,
        inPlace: true,
        relaunching: false,
        reloaded: false,
        mode,
        reason: error instanceof Error ? error.message : String(error),
      };
    }
    return requestFullRelaunchFallback(fullRelaunch, reason, {
      reason: error instanceof Error ? error.message : String(error),
      broadcastStatus,
      logger,
      mode,
    });
  }
}

function requestFullRelaunchFallback(
  fullRelaunch,
  reason,
  { reason: fallbackReason, broadcastStatus, logger, mode = null } = {},
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
    mode,
    reason: fallback.reason ?? fallbackReason ?? reason,
  };
  broadcastStatus?.({
    lifecycle: {
      type: "rendererReload",
      phase: fallback.ok ? "fullRelaunchFallback" : "failed",
      mode,
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

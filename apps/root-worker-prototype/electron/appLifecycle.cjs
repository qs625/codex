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
  };
}

function createClientRelaunchNotificationHandler({
  rendererReload,
  installedArtifactUpdate,
} = {}) {
  return async function handleClientRelaunchNotification(notification) {
    const reason = notification?.params?.reason ?? notification?.method ?? null;
    if (
      installedArtifactUpdate &&
      typeof installedArtifactUpdate.requestUpdateAndRelaunch === "function"
    ) {
      const updateResult =
        await installedArtifactUpdate.requestUpdateAndRelaunch(reason);
      if (!updateResult.unsupported) {
        return updateResult;
      }
    }
    if (!rendererReload || typeof rendererReload.requestReload !== "function") {
      return {
        ok: false,
        inPlace: false,
        relaunching: false,
        reloaded: false,
        reason: "Renderer reload adapter is unavailable",
      };
    }
    return rendererReload.requestReload(reason);
  };
}

async function observeClientRelaunchResult(
  resultPromise,
  { broadcastStatus, logger = console, reason = null } = {},
) {
  try {
    const result = await resultPromise;
    broadcastStatus?.({
      lifecycle: {
        type: "clientRelaunch",
        phase: result?.ok ? "completed" : "failed",
        reason: result?.reason ?? reason,
      },
      relaunch: result?.relaunch ?? result ?? null,
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
        reason: message,
      },
    });
    return {
      ok: false,
      inPlace: false,
      relaunching: false,
      reloaded: false,
      updated: false,
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

  return {
    requestUpdateAndRelaunch(reason = null) {
      if (inFlight) {
        return inFlight.then((result) => ({
          ...result,
          alreadyRequested: true,
        }));
      }
      if (!resolvePlan || typeof resolvePlan !== "function") {
        return Promise.resolve({ ok: false, unsupported: true });
      }
      const plan = resolvePlan();
      if (!plan) {
        return Promise.resolve({ ok: false, unsupported: true });
      }

      inFlight = runInstalledArtifactUpdate({
        appServerRestart,
        appServerStop,
        fullRelaunch,
        reloadWindows,
        updateArtifacts,
        broadcastStatus,
        logger,
        plan,
        reason,
      }).finally(() => {
        inFlight = null;
      });
      return inFlight;
    },
  };
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
}) {
  if (!updateArtifacts || typeof updateArtifacts !== "function") {
    return {
      ok: false,
      unsupported: false,
      relaunching: false,
      updated: false,
      reason: "Installed artifact updater is unavailable",
    };
  }

  broadcastStatus?.({
    lifecycle: {
      type: "installedArtifactUpdate",
      phase: "building",
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
        reason,
      },
    });
    if (plan.requiresFullRelaunch) {
      return await relaunchUpdatedApp({
        appServerStop,
        broadcastStatus,
        fullRelaunch,
        reason,
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
      reason,
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
  reason,
  update,
}) {
  broadcastStatus?.({
    lifecycle: {
      type: "installedArtifactUpdate",
      phase: "relaunching",
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

async function reloadUpdatedRenderer(reloadWindows, { broadcastStatus, reason }) {
  if (typeof reloadWindows !== "function") {
    return {
      ok: false,
      inPlace: true,
      relaunching: false,
      reloaded: false,
      reason: "Renderer reload is unavailable after installed artifact update",
    };
  }
  broadcastStatus?.({
    lifecycle: {
      type: "installedArtifactUpdate",
      phase: "reloading",
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
      reason,
    };
  } catch (error) {
    return {
      ok: false,
      inPlace: true,
      relaunching: false,
      reloaded: false,
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

async function runRendererReload({
  fullRelaunch,
  reloadWindows,
  broadcastStatus,
  logger,
  reason,
}) {
  if (typeof reloadWindows !== "function") {
    return requestFullRelaunchFallback(fullRelaunch, reason, {
      reason: "Renderer reload is unavailable in this environment",
      broadcastStatus,
      logger,
    });
  }

  broadcastStatus?.({
    lifecycle: {
      type: "rendererReload",
      phase: "reloading",
      reason,
    },
  });

  try {
    const reload = await reloadWindows({ reason });
    broadcastStatus?.({
      lifecycle: {
        type: "rendererReload",
        phase: "reloaded",
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
      reason,
    };
  } catch (error) {
    return requestFullRelaunchFallback(fullRelaunch, reason, {
      reason: error instanceof Error ? error.message : String(error),
      broadcastStatus,
      logger,
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

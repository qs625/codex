function createAppRelaunchAdapter({
  app,
  setTimeout: scheduleExit = setTimeout,
  exitDelayMs = 50,
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
      scheduleExit(() => app.exit(0), exitDelayMs);
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

function createInstalledArtifactUpdateLifecycleAdapter({
  fullRelaunch,
  resolvePlan,
  updateArtifacts,
  broadcastStatus,
  logger = console,
} = {}) {
  let inFlight = null;

  return {
    requestUpdateAndRelaunch(reason = null) {
      if (!resolvePlan || typeof resolvePlan !== "function") {
        return Promise.resolve({ ok: false, unsupported: true });
      }
      const plan = resolvePlan();
      if (!plan) {
        return Promise.resolve({ ok: false, unsupported: true });
      }
      if (inFlight) {
        return inFlight.then((result) => ({
          ...result,
          alreadyRequested: true,
        }));
      }

      inFlight = runInstalledArtifactUpdate({
        fullRelaunch,
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
  fullRelaunch,
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
    const relaunch =
      fullRelaunch && typeof fullRelaunch.requestRelaunch === "function"
        ? fullRelaunch.requestRelaunch(reason)
        : {
            ok: false,
            relaunching: false,
            reason: "Application relaunch is unavailable in this environment",
          };
    broadcastStatus?.({
      lifecycle: {
        type: "installedArtifactUpdate",
        phase: relaunch.ok ? "relaunching" : "failed",
        reason: relaunch.reason ?? reason,
      },
      relaunch,
    });
    return {
      ok: Boolean(update.ok && relaunch.ok),
      inPlace: false,
      relaunching: Boolean(relaunch.relaunching),
      reloaded: false,
      updated: Boolean(update.updated),
      relaunch,
      reason: relaunch.reason ?? reason,
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
};

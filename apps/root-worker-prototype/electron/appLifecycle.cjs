const FULL_ACTIVATION_EXIT_CODE = 75;
const RECOVERY_RESTART_EXIT_CODE = 76;

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
    const requestedByThreadId = requestedByThreadIdFromNotification(notification);
    if (!requestedByThreadId.ok) {
      return {
        ok: false,
        inPlace: false,
        relaunching: false,
        reloaded: false,
        updated: false,
        mode,
        ...requestIdFields(requestId),
        reason: requestedByThreadId.reason,
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
      requestedByThreadId: requestedByThreadId.value,
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
  requestedByThreadId,
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
        requestedByThreadId,
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
  if (
    !rendererReload ||
    (typeof rendererReload.requestHotReload !== "function" &&
      typeof rendererReload.requestReload !== "function")
  ) {
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

function requestedByThreadIdFromNotification(notification) {
  const params = notification?.params;
  const present =
    params !== null &&
    typeof params === "object" &&
    Object.hasOwn(params, "requestedByThreadId");
  return normalizeRequestedByThreadId(params?.requestedByThreadId, present);
}

function normalizeRequestedByThreadId(value, present) {
  if (!present || value === null) {
    return { ok: true, value: null };
  }
  if (typeof value !== "string" || !value.trim()) {
    return {
      ok: false,
      reason:
        "Invalid requestedByThreadId: expected a non-empty string, null, or an omitted field",
    };
  }
  return { ok: true, value: value.trim() };
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
  appExit,
  cleanupPreparedArtifact,
  releasePreparedArtifactLease,
  reloadWindows,
  recoverLauncherFailure,
  resolvePlan,
  runtimeLauncher,
  updateArtifacts,
  broadcastStatus,
  logger = console,
} = {}) {
  let inFlight = null;
  let inFlightMode = null;

  return {
    requestUpdateAndRelaunch(
      reason = null,
      mode = null,
      requestId = null,
      requestedByThreadId,
    ) {
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
      const normalizedRequestedByThreadId = normalizeRequestedByThreadId(
        requestedByThreadId,
        arguments.length >= 4,
      );
      if (!normalizedRequestedByThreadId.ok) {
        return Promise.resolve({
          ok: false,
          unsupported: false,
          inPlace: false,
          relaunching: false,
          reloaded: false,
          updated: false,
          mode: normalizedMode,
          ...requestIdFields(requestId),
          reason: normalizedRequestedByThreadId.reason,
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
        appExit,
        cleanupPreparedArtifact,
        releasePreparedArtifactLease,
        reloadWindows,
        recoverLauncherFailure,
        runtimeLauncher,
        updateArtifacts,
        broadcastStatus,
        logger,
        resolvePlan,
        reason,
        mode: normalizedMode,
        requestId,
        requestedByThreadId: normalizedRequestedByThreadId.value,
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
  appExit,
  cleanupPreparedArtifact,
  releasePreparedArtifactLease,
  reloadWindows,
  recoverLauncherFailure,
  runtimeLauncher,
  updateArtifacts,
  broadcastStatus,
  logger,
  resolvePlan,
  reason,
  mode,
  requestId,
  requestedByThreadId,
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
    appExit,
    cleanupPreparedArtifact,
    releasePreparedArtifactLease,
    reloadWindows,
    recoverLauncherFailure,
    runtimeLauncher,
    updateArtifacts,
    broadcastStatus,
    logger,
    plan,
    reason,
    mode,
    requestId,
    requestedByThreadId,
  });
}

async function runInstalledArtifactUpdate({
  appServerRestart,
  appServerStop,
  appExit,
  cleanupPreparedArtifact,
  releasePreparedArtifactLease,
  reloadWindows,
  recoverLauncherFailure,
  runtimeLauncher,
  updateArtifacts,
  broadcastStatus,
  logger,
  plan,
  reason,
  mode,
  requestId,
  requestedByThreadId,
}) {
  let preparedUpdate = null;
  let hotPreparedArtifactCleaned = false;
  let fullPreparedArtifactLeaseReleased = false;
  async function cleanupHotPreparedRoot() {
    if (
      mode !== "hot" ||
      hotPreparedArtifactCleaned ||
      !preparedUpdate?.preparedRoot
    ) {
      return;
    }
    hotPreparedArtifactCleaned = true;
    try {
      await cleanupPreparedArtifact?.(
        preparedUpdate.preparedRoot,
        preparedUpdate.preparedArtifactsRoot,
        preparedUpdate.preparedArtifactOwner,
      );
    } catch (error) {
      logger?.warn?.(
        "[prototype] hot prepared artifact cleanup failed",
        JSON.stringify({
          reason: error instanceof Error ? error.message : String(error),
        }),
      );
    }
  }
  async function releaseFullPreparedLease() {
    if (
      mode !== "full" ||
      fullPreparedArtifactLeaseReleased ||
      !preparedUpdate?.preparedRoot
    ) {
      return;
    }
    await releasePreparedArtifactLease?.(
      preparedUpdate.preparedRoot,
      preparedUpdate.preparedArtifactsRoot,
      preparedUpdate.preparedArtifactOwner,
    );
    fullPreparedArtifactLeaseReleased = true;
  }
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
      phase: "preparing",
      mode,
      ...requestIdFields(requestId),
      reason,
    },
  });

  try {
    const update = await updateArtifacts(plan);
    preparedUpdate = update;
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
    const activationRequest = buildActivationRequest({
      update,
      mode,
      reason,
      requestId,
      requestedByThreadId,
    });
    if (mode === "full") {
      return await prepareFullRelaunch({
        appExit,
        appServerStop,
        broadcastStatus,
        runtimeLauncher,
        mode,
        reason,
        requestId,
        update,
        activationRequest,
        releasePreparedArtifactLease: releaseFullPreparedLease,
      });
    }
    assertHotActivationAllowed(plan, update);
    let activation;
    try {
      activation = await runtimeLauncher.activateHot(activationRequest);
    } catch (error) {
      throw await handleHotActivationFailure(error, {
        appBundlePath: update.appBundlePath,
        appExit,
        recoverLauncherFailure,
        runtimeLauncher,
      });
    } finally {
      await cleanupHotPreparedRoot();
    }
    if (!activation?.ok && !activation?.accepted) {
      throw new Error(
        activation?.reason ?? "Runtime launcher rejected hot activation",
      );
    }
    const backendRestart = await restartUpdatedAppServer(appServerRestart, reason);
    if (!backendRestart.ok) {
      throw await rollbackHotUpdate(
        backendRestart.reason ?? "App-server restart failed after update",
        runtimeLauncher,
        update.transactionId,
        {
          appExit,
          appServerRestart,
          appBundlePath: update.appBundlePath,
          backendRestart,
          recoverLauncherFailure,
          reloadWindows,
          reason,
          updated: true,
        },
      );
    }
    const reload = await reloadUpdatedRenderer(reloadWindows, {
      broadcastStatus,
      mode,
      reason,
      requestId,
    });
    if (!reload.ok) {
      throw await rollbackHotUpdate(
        reload.reason ?? "Renderer reload failed after update",
        runtimeLauncher,
        update.transactionId,
        {
          appExit,
          appServerRestart,
          appBundlePath: update.appBundlePath,
          backendRestart,
          recoverLauncherFailure,
          reload,
          reloadWindows,
          reason,
          updated: true,
        },
      );
    }
    let committed;
    try {
      committed = await runtimeLauncher.commitHot(update.transactionId);
    } catch (error) {
      throw await rollbackHotUpdate(
        error instanceof Error ? error.message : String(error),
        runtimeLauncher,
        update.transactionId,
        {
          appExit,
          appServerRestart,
          appBundlePath: update.appBundlePath,
          backendRestart,
          recoverLauncherFailure,
          reload,
          reloadWindows,
          reason,
          updated: true,
        },
      );
    }
    if (!committed?.ok && !committed?.committed) {
      throw await rollbackHotUpdate(
        committed?.reason ?? "Runtime launcher failed to commit hot activation",
        runtimeLauncher,
        update.transactionId,
        {
          appExit,
          appServerRestart,
          appBundlePath: update.appBundlePath,
          backendRestart,
          recoverLauncherFailure,
          reload,
          reloadWindows,
          reason,
          updated: true,
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
      updated: true,
      activation,
      committed,
      backendRestart,
      mode,
      ...requestIdFields(requestId),
      mainProcessUpdate: "unchanged",
      preloadUpdate: "unchanged",
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
      relaunching: Boolean(error?.relaunching),
      reloaded: false,
      updated: Boolean(error?.updated),
      backendRestart: error?.backendRestart,
      reload: error?.reload,
      coordinatedRestart: error?.coordinatedRestart,
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
  } finally {
    await cleanupHotPreparedRoot();
    if (mode === "full" && preparedUpdate?.preparedRoot) {
      try {
        await releaseFullPreparedLease();
      } catch (error) {
        logger?.warn?.(
          "[prototype] full prepared artifact lease release failed",
          JSON.stringify({
            reason: error instanceof Error ? error.message : String(error),
          }),
        );
      }
    }
  }
}

async function prepareFullRelaunch({
  appExit,
  appServerStop,
  broadcastStatus,
  runtimeLauncher,
  mode,
  reason,
  requestId,
  update,
  activationRequest,
  releasePreparedArtifactLease,
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
  if (!runtimeLauncher?.supported) {
    throw new Error("Stable runtime launcher is unavailable for full activation");
  }
  const prepared = await runtimeLauncher.prepareFull(activationRequest);
  if (!prepared?.ok && !prepared?.accepted) {
    throw new Error(
      prepared?.reason ?? "Runtime launcher rejected full activation",
    );
  }
  try {
    await releasePreparedArtifactLease?.();
  } catch (error) {
    throw await abortPreparedFullUpdate(
      `Prepared artifact handoff failed: ${
        error instanceof Error ? error.message : String(error)
      }`,
      runtimeLauncher,
      update.transactionId,
    );
  }
  const backendStop = await stopUpdatedAppServer(appServerStop, reason);
  if (!backendStop.ok) {
    throw await abortPreparedFullUpdate(
      backendStop.reason ?? "App-server stop failed before app relaunch",
      runtimeLauncher,
      update.transactionId,
      { backendStop },
    );
  }
  if (typeof appExit !== "function") {
    throw await abortPreparedFullUpdate(
      "Application exit is unavailable after preparing full activation",
      runtimeLauncher,
      update.transactionId,
      { backendStop },
    );
  }
  try {
    appExit(FULL_ACTIVATION_EXIT_CODE);
  } catch (error) {
    throw await abortPreparedFullUpdate(
      error instanceof Error ? error.message : String(error),
      runtimeLauncher,
      update.transactionId,
      { backendStop },
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
    relaunch: {
      ok: true,
      supervised: true,
      exitCode: FULL_ACTIVATION_EXIT_CODE,
    },
  });
  return {
    ok: true,
    inPlace: false,
    relaunching: true,
    reloaded: false,
    updated: true,
    backendStop,
    mode,
    ...requestIdFields(requestId),
    mainProcessUpdate: "requiresAppRelaunch",
    preloadUpdate: "requiresAppRelaunch",
    reason,
    relaunch: {
      ok: true,
      supervised: true,
      exitCode: FULL_ACTIVATION_EXIT_CODE,
    },
    prepared,
  };
}

async function abortPreparedFullUpdate(
  message,
  runtimeLauncher,
  transactionId,
  details = {},
) {
  let abort;
  try {
    abort = await runtimeLauncher.abortFull(transactionId);
  } catch (error) {
    abort = {
      ok: false,
      reason: error instanceof Error ? error.message : String(error),
    };
  }
  return partialInstalledUpdateError(
    abort?.ok === false
      ? `${message}; abort-full also failed: ${abort.reason ?? "unknown failure"}`
      : message,
    { ...details, abort, updated: false },
  );
}

function buildActivationRequest({
  update,
  mode,
  reason,
  requestId,
  requestedByThreadId,
}) {
  const normalizedRequestedByThreadId = normalizeRequestedByThreadId(
    requestedByThreadId,
    true,
  );
  if (!normalizedRequestedByThreadId.ok) {
    throw new TypeError(normalizedRequestedByThreadId.reason);
  }
  return {
    schemaVersion: 1,
    transactionId: update.transactionId,
    requestId,
    requestedByThreadId: normalizedRequestedByThreadId.value,
    mode,
    buildId: update.buildId,
    sourceCommit: update.sourceCommit,
    preparedRoot: update.preparedRoot,
    appBundlePath: update.appBundlePath,
    reason: typeof reason === "string" ? reason : "",
  };
}

function assertHotActivationAllowed(plan, update) {
  const changes = update?.manifest?.changes ?? {};
  if (
    plan.requiresFullRelaunch ||
    changes.main === true ||
    changes.preload === true
  ) {
    throw new Error(
      "Hot activation rejected because Electron main or preload changed; request mode=full.",
    );
  }
  if (!update?.transactionId || !update?.preparedRoot) {
    throw new Error("Prepared runtime transaction is incomplete");
  }
}

async function rollbackHotUpdate(
  message,
  runtimeLauncher,
  transactionId,
  details = {},
) {
  let rollback;
  let recovery = null;
  try {
    rollback = await runtimeLauncher.rollbackHot(
      transactionId,
      details.appBundlePath,
    );
  } catch (error) {
    rollback = {
      ok: false,
      rolledBack: error?.rolledBack === true,
      launcherResult: error?.launcherResult ?? null,
      failureEvidence: error?.failureEvidence ?? null,
      evidencePath: error?.evidencePath ?? null,
      reason: error instanceof Error ? error.message : String(error),
    };
  }
  if (rollback?.ok !== false) {
    const backendRestart = await restartUpdatedAppServer(
      details.appServerRestart,
      `rollback: ${details.reason ?? message}`,
    );
    const reload = backendRestart.ok
      ? await reloadUpdatedRenderer(details.reloadWindows, {
          mode: "hot",
          reason: `rollback: ${details.reason ?? message}`,
          requestId: null,
        })
      : null;
    recovery = { backendRestart, reload };
  }
  let evidenceRecovery = null;
  if (
    rollback?.ok !== false &&
    recovery?.backendRestart?.ok === true &&
    recovery?.reload?.ok === true &&
    rollback?.failureEvidence &&
    rollback?.evidencePath &&
    typeof details.recoverLauncherFailure === "function"
  ) {
    try {
      evidenceRecovery = await details.recoverLauncherFailure({
        evidence: rollback.failureEvidence,
        evidencePath: rollback.evidencePath,
      });
    } catch (error) {
      evidenceRecovery = {
        ok: false,
        reason: error instanceof Error ? error.message : String(error),
      };
    }
  }
  const recoveryComplete =
    rollback?.ok !== false &&
    recovery?.backendRestart?.ok === true &&
    recovery?.reload?.ok === true &&
    evidenceWasReliablyRecorded(evidenceRecovery);
  const coordinatedRestart = recoveryComplete
    ? null
    : requestCoordinatedRecoveryRestart(details.appExit);
  return partialInstalledUpdateError(
    [
      message,
      rollback?.ok === false
        ? `runtime rollback also failed: ${rollback.reason ?? "unknown failure"}`
        : null,
      evidenceRecovery?.ok === false
        ? `rollback evidence recovery failed: ${evidenceRecovery.reason}`
        : null,
      recovery?.backendRestart?.ok === false
        ? `restored app-server restart failed: ${recovery.backendRestart.reason ?? "unknown failure"}`
        : null,
      recovery?.reload?.ok === false
        ? `restored renderer reload failed: ${recovery.reload.reason ?? "unknown failure"}`
        : null,
      rollback?.ok !== false &&
      recovery?.backendRestart?.ok === true &&
      recovery?.reload?.ok === true &&
      !evidenceWasReliablyRecorded(evidenceRecovery)
        ? "rollback evidence was not reliably recorded"
        : null,
      coordinatedRestart?.ok
        ? "supervisor recovery restart requested"
        : coordinatedRestart?.reason,
    ]
      .filter(Boolean)
      .join("; "),
    {
      ...details,
      rollback,
      recovery,
      evidenceRecovery,
      coordinatedRestart,
      relaunching: coordinatedRestart?.ok === true,
    },
  );
}

function evidenceWasReliablyRecorded(result) {
  return result?.ok === true || result?.recovered === true;
}

async function handleHotActivationFailure(
  error,
  {
    appBundlePath,
    appExit,
    recoverLauncherFailure,
    runtimeLauncher,
  },
) {
  const message = error instanceof Error ? error.message : String(error);
  const launcherResult = error?.launcherResult ?? null;
  const rolledBack =
    error?.rolledBack === true ||
    launcherResult?.rolledBack === true ||
    launcherResult?.result?.rolledBack === true;
  let evidenceRecovery = null;
  if (rolledBack) {
    const failure = await resolveLauncherFailure(error, {
      appBundlePath,
      runtimeLauncher,
    });
    if (
      failure?.evidence &&
      failure?.evidencePath &&
      typeof recoverLauncherFailure === "function"
    ) {
      try {
        evidenceRecovery = await recoverLauncherFailure(failure);
      } catch (recoveryError) {
        evidenceRecovery = {
          ok: false,
          reason:
            recoveryError instanceof Error
              ? recoveryError.message
              : String(recoveryError),
        };
      }
    }
  }
  const coordinatedRestart = requestCoordinatedRecoveryRestart(appExit);
  return partialInstalledUpdateError(
    [
      message,
      evidenceRecovery?.ok === false
        ? `activation evidence recovery failed: ${evidenceRecovery.reason}`
        : null,
      coordinatedRestart?.ok ? null : coordinatedRestart?.reason,
    ]
      .filter(Boolean)
      .join("; "),
    {
      coordinatedRestart,
      evidenceRecovery,
      launcherResult,
      relaunching: coordinatedRestart?.ok === true,
      updated: false,
    },
  );
}

async function resolveLauncherFailure(error, { appBundlePath, runtimeLauncher }) {
  let failureEvidence = error?.failureEvidence ?? null;
  let evidencePath = error?.evidencePath ?? null;
  if (
    (!failureEvidence || !evidencePath) &&
    runtimeLauncher &&
    typeof runtimeLauncher.status === "function"
  ) {
    try {
      const status = await runtimeLauncher.status(appBundlePath);
      failureEvidence =
        failureEvidence ?? status?.result?.failureEvidence ?? null;
      evidencePath = evidencePath ?? status?.evidencePath ?? null;
    } catch {
      // Preserve the original activation failure. The durable evidence remains
      // available for the supervised restart to inject on the next launch.
    }
  }
  return failureEvidence && evidencePath
    ? { evidence: failureEvidence, evidencePath }
    : null;
}

function requestCoordinatedRecoveryRestart(appExit) {
  if (typeof appExit !== "function") {
    return {
      ok: false,
      supervised: false,
      reason: "Application exit is unavailable for supervised recovery restart",
    };
  }
  try {
    appExit(RECOVERY_RESTART_EXIT_CODE);
    return {
      ok: true,
      supervised: true,
      exitCode: RECOVERY_RESTART_EXIT_CODE,
    };
  } catch (error) {
    return {
      ok: false,
      supervised: false,
      reason:
        error instanceof Error
          ? `Supervised recovery restart failed: ${error.message}`
          : `Supervised recovery restart failed: ${String(error)}`,
    };
  }
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
  try {
    const result = await appServerRestart.requestRestart(reason);
    return {
      ok: Boolean(result?.ok),
      pending: false,
      restarted: Boolean(result?.restarted ?? result?.ok),
      pid: result?.pid ?? null,
      reason: result?.reason ?? reason,
    };
  } catch (error) {
    return {
      ok: false,
      pending: false,
      restarted: false,
      pid: null,
      reason: error instanceof Error ? error.message : String(error),
    };
  }
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

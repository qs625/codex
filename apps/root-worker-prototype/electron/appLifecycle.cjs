"use strict";

const { spawnSync } = require("node:child_process");
const path = require("node:path");
const {
  APP_NAME,
  COMPUTER_USE_HELPER_APP_RELATIVE_PATH,
  COMPUTER_USE_STABLE_HELPER_APP_PATH,
  PAYLOAD_RELATIVE_PATH,
  materializeStableComputerUseHelperApp,
} = require("./installedArtifactUpdate.cjs");

// Hand the selected Runtime Capsule back to its supervising launcher. This is
// distinct from exit 0, which means the user intentionally closed the app.
const CAPSULE_SWITCH_EXIT_CODE = 75;

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
    const handoff =
      notification?.params?.runtimeRestartHandoff &&
      typeof notification.params.runtimeRestartHandoff === "object"
        ? notification.params.runtimeRestartHandoff
        : {};
    const requestId = normalizeClientRelaunchRequestId(
      notification?.params?.requestId,
    );
    if (Object.hasOwn(notification?.params ?? {}, "mode")) {
      return Promise.resolve({
        ok: false,
        unsupported: true,
        inPlace: false,
        relaunching: false,
        reloaded: false,
        updated: false,
        ...requestIdFields(requestId),
        reason: "Runtime restart requests do not support mode.",
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
      markExpectedRestartHandoffReady:
        typeof handoff.markExpectedRestartHandoffReady === "function"
          ? handoff.markExpectedRestartHandoffReady
          : null,
    }).finally(() => {
      inFlight = null;
    });
    return inFlight;
  };
}

async function runClientRelaunchNotification({
  fullRelaunch,
  installedArtifactUpdate,
  markExpectedRestartHandoffReady,
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
      { markExpectedRestartHandoffReady },
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
  cleanupPreparedArtifact,
  resolvePlan,
  runtimeLauncher,
  updateArtifacts,
  materializeComputerUseHelper = materializeSelectedComputerUseHelperApp,
  runCommand = runLifecycleCommand,
  broadcastStatus,
  gracefulShutdownAppServer,
  logger = console,
} = {}) {
  let inFlight = null;
  return {
    requestUpdateAndRelaunch(reason = null, requestId = null, options = {}) {
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
        cleanupPreparedArtifact,
        gracefulShutdownAppServer,
        resolvePlan,
        runtimeLauncher,
        updateArtifacts,
        materializeComputerUseHelper,
        runCommand,
        broadcastStatus,
        logger,
        reason,
        requestId,
        markExpectedRestartHandoffReady:
          typeof options?.markExpectedRestartHandoffReady === "function"
            ? options.markExpectedRestartHandoffReady
            : null,
      }).finally(() => {
        inFlight = null;
      });
      return inFlight;
    },
  };
}

async function resolveAndRunInstalledArtifactUpdate({
  appExit,
  cleanupPreparedArtifact,
  resolvePlan,
  runtimeLauncher,
  updateArtifacts,
  materializeComputerUseHelper,
  runCommand,
  broadcastStatus,
  gracefulShutdownAppServer,
  logger,
  reason,
  requestId,
  markExpectedRestartHandoffReady,
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
    cleanupPreparedArtifact,
    plan,
    runtimeLauncher,
    updateArtifacts,
    materializeComputerUseHelper,
    runCommand,
    broadcastStatus,
    logger,
    reason,
    requestId,
    markExpectedRestartHandoffReady,
    gracefulShutdownAppServer,
  });
}

async function runInstalledArtifactUpdate({
  appExit,
  cleanupPreparedArtifact,
  plan,
  runtimeLauncher,
  updateArtifacts,
  materializeComputerUseHelper,
  runCommand,
  broadcastStatus,
  gracefulShutdownAppServer,
  logger,
  reason,
  requestId,
  markExpectedRestartHandoffReady,
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
  let selected = null;
  let computerUseHelper = null;
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
    selected = await runtimeLauncher.selectCandidate({
      activationId: update.activationId,
      reason,
      target: update.manifest?.target,
    });
    if (
      selected?.activationId !== update.activationId ||
      selected?.releaseId !== update.releaseId
    ) {
      throw new Error(
        "Runtime Capsule selection result does not match the produced candidate",
      );
    }
    try {
      computerUseHelper = await materializeComputerUseHelper?.({
        plan,
        runCommand,
        runtimeLauncher,
        selected,
        update,
      });
    } catch (error) {
      computerUseHelper = {
        status: "failed",
        reason: error instanceof Error ? error.message : String(error),
      };
      logger?.warn?.(
        "[prototype] stable Computer Use helper materialization failed",
        JSON.stringify({ reason: computerUseHelper.reason }),
      );
    }
    broadcastStatus?.({
      lifecycle: {
        type: "installedArtifactUpdate",
        phase: "selected",
        activationId: update.activationId,
        releaseId: update.releaseId,
        ...(computerUseHelper ? { computerUseHelper } : {}),
        ...requestIdFields(requestId),
        reason,
      },
    });
    if (typeof appExit !== "function") {
      throw new Error("Application exit is unavailable after Runtime Capsule selection");
    }
    if (typeof gracefulShutdownAppServer === "function") {
      broadcastStatus?.({
        lifecycle: {
          type: "installedArtifactUpdate",
          phase: "shuttingDownAppServer",
          activationId: update.activationId,
          releaseId: update.releaseId,
          ...requestIdFields(requestId),
          reason,
        },
      });
      const shutdown = await gracefulShutdownAppServer(
        reason ?? "Runtime Capsule switch",
      );
      if (shutdown && typeof shutdown === "object" && shutdown.ok === false) {
        throw new Error(
          shutdown.reason ?? "app-server graceful shutdown did not complete",
        );
      }
    }
    if (typeof markExpectedRestartHandoffReady === "function") {
      await markExpectedRestartHandoffReady();
    }
    appExit(CAPSULE_SWITCH_EXIT_CODE);
    const relaunch = {
      ok: true,
      relaunching: true,
      supervised: true,
      exitCode: CAPSULE_SWITCH_EXIT_CODE,
    };
    broadcastStatus?.({
      lifecycle: {
        type: "installedArtifactUpdate",
      phase: "exiting",
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
      selected,
      ...(computerUseHelper ? { computerUseHelper } : {}),
      relaunch,
      ...requestIdFields(requestId),
      reason,
    };
  } catch (error) {
    if (update?.incomingRoot && !selected) {
      await cleanupUnselectedCandidate(
        cleanupPreparedArtifact,
        update.incomingRoot,
        logger,
      );
    }
    return installedUpdateFailure(error, {
      broadcastStatus,
      logger,
      phase: selected ? "selected" : "preparing",
      reason,
      requestId,
      update,
    });
  }
}

async function cleanupUnselectedCandidate(cleanup, incomingRoot, logger) {
  if (!incomingRoot || typeof cleanup !== "function") {
    return;
  }
  try {
    await cleanup(incomingRoot);
  } catch (error) {
    logger?.warn?.(
      "[prototype] Runtime Capsule cleanup failed",
      JSON.stringify({
        reason: error instanceof Error ? error.message : String(error),
      }),
    );
  }
}

async function materializeSelectedComputerUseHelperApp({
  runCommand,
  runtimeLauncher,
  selected,
  update,
} = {}) {
  if (update?.computerUseHelper?.status !== "pending-selection") {
    return update?.computerUseHelper ?? null;
  }
  const selectedRoot = selectedRuntimeCapsuleRoot(selected, runtimeLauncher);
  if (!selectedRoot) {
    return {
      status: "skipped",
      reason: "selected Runtime Capsule root is unavailable",
      targetAppPath:
        update.computerUseHelper.targetAppPath ??
        COMPUTER_USE_STABLE_HELPER_APP_PATH,
    };
  }
  const payloadAppPath = path.join(
    selectedRoot,
    ...PAYLOAD_RELATIVE_PATH.split(path.sep),
  );
  return materializeStableComputerUseHelperApp({
    payloadElectronPath: path.join(
      payloadAppPath,
      "Contents",
      "MacOS",
      APP_NAME,
    ),
    sourceAppPath: path.join(
      payloadAppPath,
      "Contents",
      "Resources",
      ...COMPUTER_USE_HELPER_APP_RELATIVE_PATH.split(path.sep),
    ),
    runCommand,
    targetAppPath:
      update.computerUseHelper.targetAppPath ??
      COMPUTER_USE_STABLE_HELPER_APP_PATH,
  });
}

function selectedRuntimeCapsuleRoot(selected, runtimeLauncher) {
  const selectedCapsuleRoot = normalizeString(
    selected?.control?.selected?.capsule?.root,
  );
  if (selectedCapsuleRoot) {
    return selectedCapsuleRoot;
  }
  const digest = releaseDigest(selected?.releaseId);
  const stateRoot = normalizeString(runtimeLauncher?.stateRoot);
  return digest && stateRoot ? path.join(stateRoot, "artifacts", digest) : null;
}

function releaseDigest(releaseId) {
  const value = normalizeString(releaseId);
  const match = /^sha256:([0-9a-f]{64})$/.exec(value ?? "");
  return match?.[1] ?? null;
}

function runLifecycleCommand(command, args, options = {}) {
  const result = (options.spawnSync ?? spawnSync)(command, args, {
    cwd: options.cwd,
    encoding: "utf8",
    env: options.env,
    stdio: options.stdio ?? "pipe",
  });
  if (result?.error) {
    throw result.error;
  }
  if (result?.status !== 0) {
    const stderr = normalizeString(result?.stderr);
    throw new Error(
      `${command} ${args.join(" ")} exited with ${String(result?.status)}${stderr ? `: ${stderr}` : ""}`,
    );
  }
  return result.stdout ?? "";
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
    ...requestIdFields(requestId),
    reason: message,
  };
}

function normalizeClientRelaunchRequestId(requestId) {
  return typeof requestId === "string" && requestId.trim()
    ? requestId.trim()
    : null;
}

function normalizeString(value) {
  return typeof value === "string" && value.trim() ? value.trim() : null;
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
  CAPSULE_SWITCH_EXIT_CODE,
  createAppRelaunchAdapter,
  createClientRelaunchNotificationHandler,
  createInstalledArtifactUpdateLifecycleAdapter,
  createRendererReloadLifecycleAdapter,
  isClientRelaunchNotification,
  observeClientRelaunchResult,
};

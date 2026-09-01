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

module.exports = {
  createAppRelaunchAdapter,
  isClientRelaunchNotification,
};

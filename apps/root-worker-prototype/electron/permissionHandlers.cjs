function allowBrowserPanelPermission() {
  return true;
}

function allowDefaultSessionPermission({
  webContents,
  permission,
  isBrowserPanelWebContents,
}) {
  return permission === "media" && !isBrowserPanelWebContents(webContents);
}

function configurePermissionHandlers(targetSession, isAllowed) {
  targetSession.setPermissionCheckHandler(
    (webContents, permission, requestingOrigin, details) =>
      Boolean(
        isAllowed({
          webContents,
          permission,
          requestingOrigin,
          details,
          requestType: "check",
        }),
      ),
  );
  targetSession.setPermissionRequestHandler(
    (webContents, permission, callback, details) => {
      callback(
        Boolean(
          isAllowed({
            webContents,
            permission,
            details,
            requestType: "request",
          }),
        ),
      );
    },
  );
}

module.exports = {
  allowBrowserPanelPermission,
  allowDefaultSessionPermission,
  configurePermissionHandlers,
};

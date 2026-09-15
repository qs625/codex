const { contextBridge, ipcRenderer } = require("electron");

contextBridge.exposeInMainWorld("codexDesktop", {
  health: () => ipcRenderer.invoke("codex:health"),
  showSystemNotification: (payload) =>
    ipcRenderer.invoke("codex:showSystemNotification", payload),
  relaunchApp: (payload) => ipcRenderer.invoke("codex:relaunchApp", payload),
  bootstrap: () => ipcRenderer.invoke("codex:bootstrap"),
  listThreads: (cwd) => ipcRenderer.invoke("codex:listThreads", cwd),
  listModels: () => ipcRenderer.invoke("codex:listModels"),
  readConfig: (payload) => ipcRenderer.invoke("codex:readConfig", payload),
  writeConfigValue: (payload) =>
    ipcRenderer.invoke("codex:writeConfigValue", payload),
  batchWriteConfig: (payload) =>
    ipcRenderer.invoke("codex:batchWriteConfig", payload),
  readAccount: (payload) => ipcRenderer.invoke("codex:readAccount", payload),
  getAndroidConnectionInfo: () =>
    ipcRenderer.invoke("codex:androidConnectionInfo"),
  startAccountLogin: (payload) =>
    ipcRenderer.invoke("codex:startAccountLogin", payload),
  cancelAccountLogin: (payload) =>
    ipcRenderer.invoke("codex:cancelAccountLogin", payload),
  listAgentTypes: (cwd) => ipcRenderer.invoke("codex:listAgentTypes", cwd),
  listThreadProviders: (cwd) =>
    ipcRenderer.invoke("codex:listThreadProviders", cwd),
  selectProjectDirectory: (defaultPath) =>
    ipcRenderer.invoke("codex:selectProjectDirectory", defaultPath),
  listSkills: (cwd) => ipcRenderer.invoke("codex:listSkills", cwd),
  listWorkflows: (cwd) => ipcRenderer.invoke("codex:listWorkflows", cwd),
  createThread: (payload) => ipcRenderer.invoke("codex:createThread", payload),
  getSelfProject: () => ipcRenderer.invoke("codex:getSelfProject"),
  startSelfCommand: (payload) =>
    ipcRenderer.invoke("codex:startSelfCommand", payload),
  archiveThread: (threadId) =>
    ipcRenderer.invoke("codex:archiveThread", threadId),
  readThread: (threadId, includeTurns = true) =>
    ipcRenderer.invoke("codex:readThread", threadId, includeTurns),
  readCompactHistory: (threadId) =>
    ipcRenderer.invoke("codex:readCompactHistory", threadId),
  setThreadRunConfig: (payload) =>
    ipcRenderer.invoke("codex:setThreadRunConfig", payload),
  subscribeThread: (threadId) =>
    ipcRenderer.invoke("codex:subscribeThread", threadId),
  unsubscribeThread: (threadId) =>
    ipcRenderer.invoke("codex:unsubscribeThread", threadId),
  getThreadGoal: (threadId) => ipcRenderer.invoke("codex:getThreadGoal", threadId),
  setThreadGoal: (payload) => ipcRenderer.invoke("codex:setThreadGoal", payload),
  clearThreadGoal: (threadId) =>
    ipcRenderer.invoke("codex:clearThreadGoal", threadId),
  listLocalDirectory: (target) =>
    ipcRenderer.invoke("codex:listLocalDirectory", target),
  readLocalFile: (target) => ipcRenderer.invoke("codex:readLocalFile", target),
  writeLocalFile: (target, content) =>
    ipcRenderer.invoke("codex:writeLocalFile", target, content),
  readLocalImage: (target) =>
    ipcRenderer.invoke("codex:readLocalImage", target),
  readGitSnapshot: (cwd, options) =>
    ipcRenderer.invoke("codex:readGitSnapshot", cwd, options),
  readGitCommitFiles: (cwd, hash) =>
    ipcRenderer.invoke("codex:readGitCommitFiles", cwd, hash),
  readGitFileDiff: (cwd, options) =>
    ipcRenderer.invoke("codex:readGitFileDiff", cwd, options),
  readGitStatusSnapshot: (cwd) =>
    ipcRenderer.invoke("codex:readGitStatusSnapshot", cwd),
  lspDefinition: (payload) =>
    ipcRenderer.invoke("codex:lspDefinition", payload),
  lspStatus: (filePath) => ipcRenderer.invoke("codex:lspStatus", filePath),
  openLink: (target) => ipcRenderer.invoke("codex:openLink", target),
  showBrowserView: (bounds) => ipcRenderer.invoke("codex:browser:show", bounds),
  hideBrowserView: () => ipcRenderer.invoke("codex:browser:hide"),
  setBrowserViewBounds: (bounds) =>
    ipcRenderer.invoke("codex:browser:setBounds", bounds),
  navigateBrowserView: (target) =>
    ipcRenderer.invoke("codex:browser:navigate", target),
  createBrowserTab: (target) => ipcRenderer.invoke("codex:browser:newTab", target),
  selectBrowserTab: (tabId) =>
    ipcRenderer.invoke("codex:browser:selectTab", tabId),
  closeBrowserTab: (tabId) =>
    ipcRenderer.invoke("codex:browser:closeTab", tabId),
  browserGoBack: () => ipcRenderer.invoke("codex:browser:goBack"),
  browserGoForward: () => ipcRenderer.invoke("codex:browser:goForward"),
  reloadBrowserView: () => ipcRenderer.invoke("codex:browser:reload"),
  stopBrowserView: () => ipcRenderer.invoke("codex:browser:stop"),
  getTerminalState: (threadId) =>
    ipcRenderer.invoke("codex:terminal:getState", threadId),
  createTerminal: (payload) =>
    ipcRenderer.invoke("codex:terminal:create", payload),
  selectTerminalTab: (tabId) =>
    ipcRenderer.invoke("codex:terminal:select", tabId),
  focusTerminalCommand: (command) =>
    ipcRenderer.invoke("codex:terminal:focusCommand", command),
  closeTerminalTab: (tabId) =>
    ipcRenderer.invoke("codex:terminal:close", tabId),
  reattachTerminalTabs: () =>
    ipcRenderer.invoke("codex:terminal:reattach"),
  writeTerminal: (payload) =>
    ipcRenderer.invoke("codex:terminal:write", payload),
  resizeTerminal: (payload) =>
    ipcRenderer.invoke("codex:terminal:resize", payload),
  updateTerminalPreferredSize: (payload) =>
    ipcRenderer.invoke("codex:terminal:updatePreferredSize", payload),
  terminateTerminal: (tabId) =>
    ipcRenderer.invoke("codex:terminal:terminate", tabId),
  sendMessage: (payload) => ipcRenderer.invoke("codex:sendMessage", payload),
  interruptTurn: (payload) =>
    ipcRenderer.invoke("codex:interruptTurn", payload),
  respondServerRequest: (payload) =>
    ipcRenderer.invoke("codex:respondServerRequest", payload),
  rejectServerRequest: (payload) =>
    ipcRenderer.invoke("codex:rejectServerRequest", payload),
  requestMicrophoneAccess: () =>
    ipcRenderer.invoke("codex:requestMicrophoneAccess"),
  startRealtime: (payload) =>
    ipcRenderer.invoke("codex:startRealtime", payload),
  stopRealtime: (payload) => ipcRenderer.invoke("codex:stopRealtime", payload),
  subscribe(listener) {
    const onRequest = (_event, request) => {
      listener({ type: "request", request });
    };
    const onNotification = (_event, notification) => {
      listener({ type: "notification", notification });
    };
    const onStatus = (_event, status) => {
      listener({ type: "status", status });
    };

    ipcRenderer.on("codex:request", onRequest);
    ipcRenderer.on("codex:notification", onNotification);
    ipcRenderer.on("codex:status", onStatus);

    return () => {
      ipcRenderer.removeListener("codex:request", onRequest);
      ipcRenderer.removeListener("codex:notification", onNotification);
      ipcRenderer.removeListener("codex:status", onStatus);
    };
  },
  subscribeBrowserState(listener) {
    const onBrowserState = (_event, state) => {
      listener(state);
    };

    ipcRenderer.on("codex:browser:state", onBrowserState);

    return () => {
      ipcRenderer.removeListener("codex:browser:state", onBrowserState);
    };
  },
  subscribeTerminalState(listener) {
    const onTerminalState = (_event, state) => {
      listener(state);
    };

    ipcRenderer.on("codex:terminal:state", onTerminalState);

    return () => {
      ipcRenderer.removeListener("codex:terminal:state", onTerminalState);
    };
  },
});

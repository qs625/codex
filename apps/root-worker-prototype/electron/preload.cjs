const { contextBridge, ipcRenderer } = require("electron");

contextBridge.exposeInMainWorld("codexDesktop", {
  health: () => ipcRenderer.invoke("codex:health"),
  showSystemNotification: (payload) =>
    ipcRenderer.invoke("codex:showSystemNotification", payload),
  bootstrap: () => ipcRenderer.invoke("codex:bootstrap"),
  listThreads: (cwd) => ipcRenderer.invoke("codex:listThreads", cwd),
  listModels: () => ipcRenderer.invoke("codex:listModels"),
  readConfig: (payload) => ipcRenderer.invoke("codex:readConfig", payload),
  writeConfigValue: (payload) =>
    ipcRenderer.invoke("codex:writeConfigValue", payload),
  batchWriteConfig: (payload) =>
    ipcRenderer.invoke("codex:batchWriteConfig", payload),
  readAccount: (payload) => ipcRenderer.invoke("codex:readAccount", payload),
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
  getThreadGoal: (threadId) => ipcRenderer.invoke("codex:getThreadGoal", threadId),
  setThreadGoal: (payload) => ipcRenderer.invoke("codex:setThreadGoal", payload),
  clearThreadGoal: (threadId) =>
    ipcRenderer.invoke("codex:clearThreadGoal", threadId),
  listLocalDirectory: (target) =>
    ipcRenderer.invoke("codex:listLocalDirectory", target),
  readLocalFile: (target) => ipcRenderer.invoke("codex:readLocalFile", target),
  readLocalImage: (target) =>
    ipcRenderer.invoke("codex:readLocalImage", target),
  lspDefinition: (payload) =>
    ipcRenderer.invoke("codex:lspDefinition", payload),
  lspStatus: (filePath) => ipcRenderer.invoke("codex:lspStatus", filePath),
  openLink: (target) => ipcRenderer.invoke("codex:openLink", target),
  sendMessage: (payload) => ipcRenderer.invoke("codex:sendMessage", payload),
  interruptTurn: (payload) =>
    ipcRenderer.invoke("codex:interruptTurn", payload),
  requestMicrophoneAccess: () =>
    ipcRenderer.invoke("codex:requestMicrophoneAccess"),
  startRealtime: (payload) =>
    ipcRenderer.invoke("codex:startRealtime", payload),
  stopRealtime: (payload) => ipcRenderer.invoke("codex:stopRealtime", payload),
  subscribe(listener) {
    const onNotification = (_event, notification) => {
      listener({ type: "notification", notification });
    };
    const onStatus = (_event, status) => {
      listener({ type: "status", status });
    };

    ipcRenderer.on("codex:notification", onNotification);
    ipcRenderer.on("codex:status", onStatus);

    return () => {
      ipcRenderer.removeListener("codex:notification", onNotification);
      ipcRenderer.removeListener("codex:status", onStatus);
    };
  },
});

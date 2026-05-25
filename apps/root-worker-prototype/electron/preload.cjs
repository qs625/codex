const { contextBridge, ipcRenderer } = require("electron");

contextBridge.exposeInMainWorld("codexDesktop", {
  health: () => ipcRenderer.invoke("codex:health"),
  bootstrap: () => ipcRenderer.invoke("codex:bootstrap"),
  listThreads: (cwd) => ipcRenderer.invoke("codex:listThreads", cwd),
  createThread: (payload) => ipcRenderer.invoke("codex:createThread", payload),
  archiveThread: (threadId) => ipcRenderer.invoke("codex:archiveThread", threadId),
  readThread: (threadId, subscribe = true) =>
    ipcRenderer.invoke("codex:readThread", threadId, subscribe),
  readLocalFile: (target) => ipcRenderer.invoke("codex:readLocalFile", target),
  lspDefinition: (payload) => ipcRenderer.invoke("codex:lspDefinition", payload),
  lspStatus: (filePath) => ipcRenderer.invoke("codex:lspStatus", filePath),
  openLink: (target) => ipcRenderer.invoke("codex:openLink", target),
  sendMessage: (payload) => ipcRenderer.invoke("codex:sendMessage", payload),
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

const { contextBridge, ipcRenderer } = require("electron");

function subscribeIpcState(channel, listener) {
  if (typeof listener !== "function") {
    return () => {};
  }

  const onState = (_event, state) => {
    listener(state);
  };

  ipcRenderer.on(channel, onState);

  return () => {
    ipcRenderer.removeListener(channel, onState);
  };
}

function createInvokeApi(channels) {
  return Object.fromEntries(
    Object.entries(channels).map(([methodName, channel]) => [
      methodName,
      (...args) => ipcRenderer.invoke(channel, ...args),
    ]),
  );
}

function invokeChannelGroup(prefix, entries) {
  const normalizedEntries =
    typeof entries === "string" ? entries.trim().split(/\s+/) : entries;
  return Object.fromEntries(
    normalizedEntries.map((entry) => {
      const [methodName, channelName = methodName] =
        typeof entry === "string" && entry.includes(":")
          ? entry.split(":", 2)
          : Array.isArray(entry)
            ? entry
            : [entry];
      return [methodName, `${prefix}${channelName}`];
    }),
  );
}

const invokeChannels = {
  ...invokeChannelGroup(
    "codex:",
    `
      health showSystemNotification relaunchApp bootstrap listThreads listModels
      readConfig writeConfigValue batchWriteConfig readAccount
      getAndroidConnectionInfo:androidConnectionInfo startAccountLogin
      cancelAccountLogin listAgentTypes listThreadProviders selectProjectDirectory
      listSkills listWorkflows createThread getSelfProject startSelfCommand
      archiveThread readCompactHistory setThreadRunConfig subscribeThread
      unsubscribeThread getThreadGoal setThreadGoal clearThreadGoal
      listLocalDirectory readLocalFile writeLocalFile readLocalImage
      readGitSnapshot readGitCommitFiles readGitCommitFileDiff readGitFileDiff
      readGitStatusSnapshot lspDefinition lspStatus openLink sendMessage
      interruptTurn respondServerRequest rejectServerRequest requestMicrophoneAccess
      startRealtime stopRealtime
    `,
  ),
  ...invokeChannelGroup(
    "codex:browser:",
    `
      showBrowserView:show hideBrowserView:hide setBrowserViewBounds:setBounds
      navigateBrowserView:navigate createBrowserTab:newTab selectBrowserTab:selectTab
      closeBrowserTab:closeTab browserGoBack:goBack browserGoForward:goForward
      reloadBrowserView:reload stopBrowserView:stop
    `,
  ),
  ...invokeChannelGroup(
    "codex:terminal:",
    `
      getTerminalState:getState createTerminal:create selectTerminalTab:select
      focusTerminalCommand:focusCommand closeTerminalTab:close
      reattachTerminalTabs:reattach writeTerminal:write resizeTerminal:resize
      updateTerminalPreferredSize:updatePreferredSize terminateTerminal:terminate
    `,
  ),
  ...invokeChannelGroup(
    "codex:computerUse:",
    `
      startComputerUse:start observeComputerUse:observe actComputerUse:act
      stopComputerUse:stop getComputerUseState:state
    `,
  ),
};

contextBridge.exposeInMainWorld("codexDesktop", {
  ...createInvokeApi(invokeChannels),
  readThread: (threadId, includeTurns = true) =>
    ipcRenderer.invoke("codex:readThread", threadId, includeTurns),
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
    return subscribeIpcState("codex:browser:state", listener);
  },
  subscribeTerminalState(listener) {
    return subscribeIpcState("codex:terminal:state", listener);
  },
});

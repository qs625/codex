function subscribeIpcState(ipcRenderer, channel, listener) {
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

module.exports = {
  subscribeIpcState,
};

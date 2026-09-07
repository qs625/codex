"use strict";

const browserSessionPartition = "persist:root-worker-browser";

function browserPanelWebPreferences() {
  return {
    allowRunningInsecureContent: false,
    contextIsolation: true,
    nodeIntegration: false,
    partition: browserSessionPartition,
    sandbox: true,
    webSecurity: true,
  };
}

module.exports = {
  browserPanelWebPreferences,
  browserSessionPartition,
};

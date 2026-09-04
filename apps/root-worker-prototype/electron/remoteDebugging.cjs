"use strict";

const REMOTE_DEBUGGING_PORT_ENV = "ROOT_WORKER_REMOTE_DEBUGGING_PORT";
const REMOTE_DEBUGGING_ADDRESS = "127.0.0.1";

function parseRemoteDebuggingConfig(env = process.env) {
  const rawPort = env[REMOTE_DEBUGGING_PORT_ENV];
  if (rawPort == null || String(rawPort).trim() === "") {
    return { enabled: false };
  }

  const portText = String(rawPort).trim();
  if (!/^\d+$/.test(portText)) {
    return invalidConfig(portText);
  }

  const port = Number(portText);
  if (!Number.isSafeInteger(port) || port < 1 || port > 65535) {
    return invalidConfig(portText);
  }

  return {
    enabled: true,
    port: String(port),
    address: REMOTE_DEBUGGING_ADDRESS,
    cdpUrl: `http://${REMOTE_DEBUGGING_ADDRESS}:${port}`,
  };
}

function applyRemoteDebuggingConfig(app, env = process.env, logger = console) {
  const config = parseRemoteDebuggingConfig(env);
  if (!config.enabled) {
    if (config.warning) {
      logger.warn(config.warning);
    }
    return config;
  }

  app.commandLine.appendSwitch("remote-debugging-port", config.port);
  app.commandLine.appendSwitch("remote-debugging-address", config.address);
  logger.error(
    "[prototype] remote debugging enabled",
    JSON.stringify({
      address: config.address,
      port: config.port,
      cdpUrl: config.cdpUrl,
    }),
  );
  return config;
}

function invalidConfig(value) {
  return {
    enabled: false,
    warning:
      `[prototype] ignoring invalid ${REMOTE_DEBUGGING_PORT_ENV}: ` +
      `${JSON.stringify(value)}; expected 1..65535`,
  };
}

module.exports = {
  REMOTE_DEBUGGING_ADDRESS,
  REMOTE_DEBUGGING_PORT_ENV,
  applyRemoteDebuggingConfig,
  parseRemoteDebuggingConfig,
};

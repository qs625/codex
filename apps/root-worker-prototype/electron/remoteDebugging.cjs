"use strict";

const REMOTE_DEBUGGING_PORT_ENV = "ROOT_WORKER_REMOTE_DEBUGGING_PORT";
const REMOTE_DEBUGGING_DISABLE_ENV = "ROOT_WORKER_DISABLE_CDP";
const REMOTE_DEBUGGING_ADDRESS = "127.0.0.1";
const DEFAULT_REMOTE_DEBUGGING_PORT = "9222";

function parseRemoteDebuggingConfig(env = process.env) {
  if (isTruthyEnv(env[REMOTE_DEBUGGING_DISABLE_ENV])) {
    return { enabled: false };
  }

  const rawPort = env[REMOTE_DEBUGGING_PORT_ENV];
  if (rawPort == null || String(rawPort).trim() === "") {
    return enabledConfig(DEFAULT_REMOTE_DEBUGGING_PORT);
  }

  const portText = String(rawPort).trim();
  if (!/^\d+$/.test(portText)) {
    return invalidConfig(portText);
  }

  const port = Number(portText);
  if (!Number.isSafeInteger(port) || port < 1 || port > 65535) {
    return invalidConfig(portText);
  }

  return enabledConfig(String(port));
}

function enabledConfig(port) {
  return {
    enabled: true,
    port,
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

function isTruthyEnv(value) {
  if (value == null) {
    return false;
  }
  return ["1", "true", "yes", "on"].includes(String(value).trim().toLowerCase());
}

module.exports = {
  DEFAULT_REMOTE_DEBUGGING_PORT,
  REMOTE_DEBUGGING_ADDRESS,
  REMOTE_DEBUGGING_DISABLE_ENV,
  REMOTE_DEBUGGING_PORT_ENV,
  applyRemoteDebuggingConfig,
  parseRemoteDebuggingConfig,
};

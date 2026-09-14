"use strict";

const REMOTE_DEBUGGING_PORT_ENV = "ROOT_WORKER_REMOTE_DEBUGGING_PORT";
const REMOTE_DEBUGGING_BACKEND_PORT_ENV =
  "ROOT_WORKER_REMOTE_DEBUGGING_BACKEND_PORT";
const REMOTE_DEBUGGING_DISABLE_ENV = "ROOT_WORKER_DISABLE_CDP";
const REMOTE_DEBUGGING_ADDRESS = "127.0.0.1";
const DEFAULT_REMOTE_DEBUGGING_PORT = "9222";

function parseRemoteDebuggingConfig(env = process.env) {
  if (isTruthyEnv(env[REMOTE_DEBUGGING_DISABLE_ENV])) {
    return { enabled: false };
  }

  const rawPort = env[REMOTE_DEBUGGING_PORT_ENV];
  if (rawPort == null || String(rawPort).trim() === "") {
    return enabledConfig(DEFAULT_REMOTE_DEBUGGING_PORT, env);
  }

  const portText = String(rawPort).trim();
  if (!/^\d+$/.test(portText)) {
    return invalidConfig(portText);
  }

  const port = Number(portText);
  if (!Number.isSafeInteger(port) || port < 1 || port > 65535) {
    return invalidConfig(portText);
  }

  return enabledConfig(String(port), env);
}

function enabledConfig(port, env = process.env) {
  const backendPort = resolveBackendPort(port, env);
  if (!backendPort.ok) {
    return invalidBackendConfig(backendPort.value);
  }
  return {
    enabled: true,
    port,
    backendPort: backendPort.port,
    address: REMOTE_DEBUGGING_ADDRESS,
    cdpUrl: `http://${REMOTE_DEBUGGING_ADDRESS}:${port}`,
    backendCdpUrl: `http://${REMOTE_DEBUGGING_ADDRESS}:${backendPort.port}`,
    proxy: {
      enabled: true,
      port,
      backendPort: backendPort.port,
      address: REMOTE_DEBUGGING_ADDRESS,
    },
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

  app.commandLine.appendSwitch("remote-debugging-port", config.backendPort);
  app.commandLine.appendSwitch("remote-debugging-address", config.address);
  logger.error(
    "[prototype] remote debugging enabled",
    JSON.stringify({
      address: config.address,
      port: config.port,
      cdpUrl: config.cdpUrl,
      backendPort: config.backendPort,
      backendCdpUrl: config.backendCdpUrl,
    }),
  );
  return config;
}

function resolveBackendPort(port, env) {
  const override = env[REMOTE_DEBUGGING_BACKEND_PORT_ENV];
  if (override != null && String(override).trim() !== "") {
    const value = String(override).trim();
    const parsed = parsePort(value);
    if (!parsed.ok || String(parsed.port) === String(port)) {
      return { ok: false, value };
    }
    return { ok: true, port: String(parsed.port) };
  }

  const externalPort = Number(port);
  const backendPort = externalPort < 65535 ? externalPort + 1 : externalPort - 1;
  return { ok: true, port: String(backendPort) };
}

function parsePort(value) {
  if (!/^\d+$/.test(value)) {
    return { ok: false };
  }
  const port = Number(value);
  if (!Number.isSafeInteger(port) || port < 1 || port > 65535) {
    return { ok: false };
  }
  return { ok: true, port };
}

function invalidConfig(value) {
  return {
    enabled: false,
    warning:
      `[prototype] ignoring invalid ${REMOTE_DEBUGGING_PORT_ENV}: ` +
      `${JSON.stringify(value)}; expected 1..65535`,
  };
}

function invalidBackendConfig(value) {
  return {
    enabled: false,
    warning:
      `[prototype] ignoring invalid ${REMOTE_DEBUGGING_BACKEND_PORT_ENV}: ` +
      `${JSON.stringify(value)}; expected 1..65535 and different from ` +
      `${REMOTE_DEBUGGING_PORT_ENV}`,
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
  REMOTE_DEBUGGING_BACKEND_PORT_ENV,
  REMOTE_DEBUGGING_DISABLE_ENV,
  REMOTE_DEBUGGING_PORT_ENV,
  applyRemoteDebuggingConfig,
  parseRemoteDebuggingConfig,
};

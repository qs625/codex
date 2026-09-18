"use strict";

const fs = require("node:fs");
const net = require("node:net");
const os = require("node:os");
const path = require("node:path");
const { randomUUID } = require("node:crypto");

const COMPUTER_USE_OVERLAY_SOCKET_ENV =
  "MORPHEUS_COMPUTER_USE_OVERLAY_SOCKET_PATH";
const DEFAULT_OVERLAY_BRIDGE_TIMEOUT_MS = 3000;

function defaultComputerUseOverlaySocketPath({
  tmpDir = os.tmpdir(),
  uid = typeof process.getuid === "function" ? process.getuid() : "nouid",
  pid = process.pid,
} = {}) {
  return path.join(
    tmpDir,
    `morpheus-computer-use-overlay-${uid}`,
    `overlay-${pid}.sock`,
  );
}

function createComputerUseOverlayBridgeServer({
  socketPath = defaultComputerUseOverlaySocketPath(),
  overlayControllerFactory,
  beforeUpdate = null,
  logger = console,
} = {}) {
  if (typeof overlayControllerFactory !== "function") {
    throw new Error("Computer Use overlay bridge requires an overlay controller factory");
  }
  let controller = null;
  let server = null;
  let started = false;

  async function controllerForRequest() {
    if (!controller) {
      controller = await overlayControllerFactory();
    }
    return controller;
  }

  async function handleMessage(message) {
    if (!message || typeof message !== "object") {
      throw new Error("Overlay bridge message must be an object");
    }
    if (message.type === "update") {
      await beforeUpdate?.();
      const overlay = await controllerForRequest();
      const result = await overlay.update(message.payload ?? {});
      return result ?? { available: true, visible: true };
    }
    if (message.type === "destroy") {
      await controller?.destroy?.();
      controller = null;
      return { available: true, visible: false, destroyed: true };
    }
    throw new Error(`Unsupported overlay bridge message: ${String(message.type)}`);
  }

  async function start() {
    if (started) {
      return { socketPath };
    }
    prepareOverlaySocketPath(socketPath);
    server = net.createServer((socket) => {
      let buffer = "";
      socket.setEncoding("utf8");
      socket.on("data", (chunk) => {
        buffer += chunk;
        while (buffer.includes("\n")) {
          const index = buffer.indexOf("\n");
          const line = buffer.slice(0, index);
          buffer = buffer.slice(index + 1);
          if (!line.trim()) {
            continue;
          }
          void respondToLine(socket, line, handleMessage);
        }
      });
    });
    server.on("error", (error) => {
      logger.warn?.("Computer Use overlay bridge error", error);
    });
    await new Promise((resolve, reject) => {
      server.once("error", reject);
      server.listen(socketPath, () => {
        server.off("error", reject);
        started = true;
        resolve();
      });
    });
    return { socketPath };
  }

  async function close() {
    await controller?.destroy?.();
    controller = null;
    if (!server) {
      return;
    }
    await new Promise((resolve) => server.close(() => resolve()));
    server = null;
    started = false;
    try {
      fs.rmSync(socketPath, { force: true });
    } catch {}
  }

  return {
    close,
    get socketPath() {
      return socketPath;
    },
    start,
  };
}

function createComputerUseOverlayBridgeClient({
  socketPath = process.env[COMPUTER_USE_OVERLAY_SOCKET_ENV],
  timeoutMs = DEFAULT_OVERLAY_BRIDGE_TIMEOUT_MS,
} = {}) {
  if (!socketPath) {
    return createUnavailableOverlayClient(
      "Computer Use overlay bridge is unavailable for this session.",
    );
  }
  return {
    async update(payload) {
      return requestOverlayBridge(socketPath, { type: "update", payload }, timeoutMs);
    },
    async destroy() {
      return requestOverlayBridge(socketPath, { type: "destroy" }, timeoutMs);
    },
  };
}

function createUnavailableOverlayClient(reason) {
  return {
    async update() {
      return { available: false, visible: false, reason };
    },
    async destroy() {
      return { available: false, visible: false, reason };
    },
  };
}

async function requestOverlayBridge(socketPath, message, timeoutMs) {
  try {
    validateOverlaySocketPath(socketPath);
  } catch (error) {
    return unavailableResult(error);
  }
  const id = randomUUID();
  return await new Promise((resolve) => {
    const socket = net.createConnection(socketPath);
    let settled = false;
    let buffer = "";
    const timer = setTimeout(() => {
      settle({ available: false, visible: false, reason: "Computer Use overlay bridge timed out." });
    }, timeoutMs);
    function settle(result) {
      if (settled) {
        return;
      }
      settled = true;
      clearTimeout(timer);
      socket.destroy();
      resolve(result);
    }
    socket.setEncoding("utf8");
    socket.on("connect", () => {
      socket.write(`${JSON.stringify({ id, ...message })}\n`);
    });
    socket.on("data", (chunk) => {
      buffer += chunk;
      while (buffer.includes("\n")) {
        const index = buffer.indexOf("\n");
        const line = buffer.slice(0, index);
        buffer = buffer.slice(index + 1);
        if (!line.trim()) {
          continue;
        }
        try {
          const response = JSON.parse(line);
          if (response.id !== id) {
            continue;
          }
          if (response.ok) {
            const { ok, id: _id, ...result } = response;
            settle(result);
          } else {
            settle({
              available: false,
              visible: false,
              reason: response.error || "Computer Use overlay bridge returned an error.",
            });
          }
        } catch (error) {
          settle(unavailableResult(error));
        }
      }
    });
    socket.on("error", (error) => settle(unavailableResult(error)));
    socket.on("close", () => {
      settle({
        available: false,
        visible: false,
        reason: "Computer Use overlay bridge closed before responding.",
      });
    });
  });
}

async function respondToLine(socket, line, handler) {
  let message;
  try {
    message = JSON.parse(line);
    const result = await handler(message);
    socket.write(`${JSON.stringify({ id: message.id ?? null, ok: true, ...result })}\n`);
  } catch (error) {
    socket.write(
      `${JSON.stringify({
        id: message?.id ?? null,
        ok: false,
        error: errorMessage(error),
      })}\n`,
    );
  }
}

function prepareOverlaySocketPath(socketPath) {
  if (!path.isAbsolute(socketPath)) {
    throw new Error("Computer Use overlay bridge socket path must be absolute");
  }
  const dir = path.dirname(socketPath);
  fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
  const stat = fs.lstatSync(dir);
  if (!stat.isDirectory() || stat.isSymbolicLink()) {
    throw new Error("Computer Use overlay bridge socket parent must be a real directory");
  }
  if (typeof process.getuid === "function" && stat.uid !== process.getuid()) {
    throw new Error("Computer Use overlay bridge socket parent must be owned by the current user");
  }
  if ((stat.mode & 0o077) !== 0) {
    fs.chmodSync(dir, 0o700);
  }
  try {
    fs.rmSync(socketPath, { force: true });
  } catch {}
}

function validateOverlaySocketPath(socketPath) {
  if (!path.isAbsolute(socketPath)) {
    throw new Error("Computer Use overlay bridge socket path must be absolute");
  }
  const dir = path.dirname(socketPath);
  const dirStat = fs.lstatSync(dir);
  if (!dirStat.isDirectory() || dirStat.isSymbolicLink()) {
    throw new Error("Computer Use overlay bridge socket parent must be a real directory");
  }
  if (typeof process.getuid === "function" && dirStat.uid !== process.getuid()) {
    throw new Error("Computer Use overlay bridge socket parent must be owned by the current user");
  }
  if ((dirStat.mode & 0o077) !== 0) {
    throw new Error("Computer Use overlay bridge socket parent must not be group/world accessible");
  }
  const socketStat = fs.lstatSync(socketPath);
  if (!socketStat.isSocket()) {
    throw new Error("Computer Use overlay bridge path is not a Unix socket");
  }
  if (typeof process.getuid === "function" && socketStat.uid !== process.getuid()) {
    throw new Error("Computer Use overlay bridge socket must be owned by the current user");
  }
}

function unavailableResult(error) {
  return {
    available: false,
    visible: false,
    reason: errorMessage(error),
  };
}

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}

module.exports = {
  COMPUTER_USE_OVERLAY_SOCKET_ENV,
  createComputerUseOverlayBridgeClient,
  createComputerUseOverlayBridgeServer,
  defaultComputerUseOverlaySocketPath,
  validateOverlaySocketPath,
};

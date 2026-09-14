"use strict";

const crypto = require("node:crypto");
const http = require("node:http");
const net = require("node:net");

const HOP_BY_HOP_HEADERS = new Set([
  "connection",
  "keep-alive",
  "proxy-authenticate",
  "proxy-authorization",
  "te",
  "trailer",
  "transfer-encoding",
  "upgrade",
]);

function startRemoteDebuggingProxy({
  address,
  port,
  backendPort,
  createTarget,
  logger = console,
}) {
  const sockets = new Set();
  const server = http.createServer((request, response) => {
    proxyHttpRequest({
      request,
      response,
      address,
      port,
      backendPort,
      logger,
    });
  });

  server.on("upgrade", (request, socket, head) => {
    proxyWebSocketUpgrade({
      request,
      socket,
      head,
      address,
      port,
      backendPort,
      createTarget,
      logger,
    });
  });
  server.on("connection", (socket) => {
    sockets.add(socket);
    socket.on("close", () => sockets.delete(socket));
  });

  server.on("error", (error) => {
    logger.error(
      "[prototype] remote debugging proxy failed",
      JSON.stringify({
        message: error instanceof Error ? error.message : String(error),
      }),
    );
  });

  server.listen(Number(port), address, () => {
    logger.error(
      "[prototype] remote debugging proxy enabled",
      JSON.stringify({
        cdpUrl: `http://${address}:${port}`,
        backendCdpUrl: `http://${address}:${backendPort}`,
      }),
    );
  });

  return {
    close: () => {
      for (const socket of sockets) {
        socket.destroy();
      }
      return new Promise((resolve, reject) => {
        server.close((error) => (error ? reject(error) : resolve()));
      });
    },
    server,
  };
}

function proxyHttpRequest({
  request,
  response,
  address,
  port,
  backendPort,
  logger,
}) {
  const backendRequest = http.request(
    {
      host: address,
      port: Number(backendPort),
      method: request.method,
      path: request.url,
      headers: rewriteRequestHeaders(request.headers, address, backendPort),
    },
    (backendResponse) => {
      const chunks = [];
      backendResponse.on("data", (chunk) => chunks.push(chunk));
      backendResponse.on("end", () => {
        const body = Buffer.concat(chunks);
        const rewrittenBody = rewriteDevToolsHttpBody({
          body,
          headers: backendResponse.headers,
          address,
          port,
          backendPort,
        });
        const headers = rewriteResponseHeaders(
          backendResponse.headers,
          rewrittenBody.length,
        );
        response.writeHead(backendResponse.statusCode ?? 502, headers);
        response.end(rewrittenBody);
      });
    },
  );

  backendRequest.on("error", (error) => {
    logger.warn(
      "[prototype] remote debugging proxy HTTP request failed",
      JSON.stringify({
        path: request.url,
        message: error instanceof Error ? error.message : String(error),
      }),
    );
    if (!response.headersSent) {
      response.writeHead(502, { "content-type": "text/plain; charset=utf-8" });
    }
    response.end("Remote debugging backend unavailable");
  });

  request.pipe(backendRequest);
}

function proxyWebSocketUpgrade({
  request,
  socket,
  head,
  address,
  port,
  backendPort,
  createTarget,
  logger,
}) {
  const backendSocket = net.connect(Number(backendPort), address);
  let handshake = Buffer.alloc(0);
  let handshakeComplete = false;
  const browserSocket = isBrowserWebSocketPath(request.url);
  const browserState = browserSocket ? createBrowserWebSocketState(socket) : null;
  const clientFrames = createFrameParser({
    onFrame: (frame) => {
      if (browserSocket && frame.opcode === 0x1) {
        void handleBrowserClientMessage({
          frame,
          clientSocket: socket,
          backendSocket,
          browserState,
          createTarget,
          logger,
        });
        return;
      }
      backendSocket.write(encodeWebSocketFrame(frame.payload, frame.opcode, true));
    },
  });
  const backendFrames = createFrameParser({
    onFrame: (frame) => {
      socket.write(encodeWebSocketFrame(frame.payload, frame.opcode, false));
      if (browserState && frame.opcode === 0x1) {
        browserState.markAttachedTarget(attachedTargetIdFromFrame(frame));
      }
    },
  });

  backendSocket.on("connect", () => {
    backendSocket.write(
      buildBackendUpgradeRequest(request, address, backendPort),
    );
    if (head?.length) {
      clientFrames.push(head);
    }
  });

  backendSocket.on("data", (chunk) => {
    if (!handshakeComplete) {
      handshake = Buffer.concat([handshake, chunk]);
      const headerEnd = handshake.indexOf("\r\n\r\n");
      if (headerEnd === -1) {
        return;
      }
      const header = handshake.subarray(0, headerEnd + 4);
      const rest = handshake.subarray(headerEnd + 4);
      socket.write(header);
      handshakeComplete = true;
      if (rest.length) {
        backendFrames.push(rest);
      }
      return;
    }
    backendFrames.push(chunk);
  });

  socket.on("data", (chunk) => clientFrames.push(chunk));
  socket.on("close", () => backendSocket.destroy());
  socket.on("error", () => backendSocket.destroy());
  backendSocket.on("close", () => socket.destroy());
  backendSocket.on("error", (error) => {
    logger.warn(
      "[prototype] remote debugging proxy WebSocket backend failed",
      JSON.stringify({
        path: request.url,
        message: error instanceof Error ? error.message : String(error),
      }),
    );
    socket.destroy();
  });
}

async function handleBrowserClientMessage({
  frame,
  clientSocket,
  backendSocket,
  browserState,
  createTarget,
  logger,
}) {
  const text = frame.payload.toString("utf8");
  let message;
  try {
    message = JSON.parse(text);
  } catch {
    backendSocket.write(encodeWebSocketFrame(frame.payload, frame.opcode, true));
    return;
  }

  if (message?.method !== "Target.createTarget") {
    backendSocket.write(encodeWebSocketFrame(frame.payload, frame.opcode, true));
    return;
  }

  try {
    const target = await createTarget(message.params?.url);
    browserState.resolveCreateTargetAfterAttach(message.id, target.targetId);
  } catch (error) {
    const messageText = error instanceof Error ? error.message : String(error);
    logger.warn(
      "[prototype] Target.createTarget rejected",
      JSON.stringify({ message: messageText }),
    );
    clientSocket.write(
      encodeWebSocketText({
        id: message.id,
        error: {
          code: -32000,
          message: messageText,
        },
      }),
    );
  }
}

function createBrowserWebSocketState(clientSocket) {
  const attachedTargets = new Set();
  const pendingCreateTargetResponses = new Map();
  return {
    markAttachedTarget(targetId) {
      if (!targetId) {
        return;
      }
      attachedTargets.add(targetId);
      const pending = pendingCreateTargetResponses.get(targetId);
      if (!pending) {
        return;
      }
      pendingCreateTargetResponses.delete(targetId);
      clearTimeout(pending.timeout);
      writeCreateTargetResponse(clientSocket, pending.id, targetId);
    },
    resolveCreateTargetAfterAttach(id, targetId) {
      if (attachedTargets.has(targetId)) {
        writeCreateTargetResponse(clientSocket, id, targetId);
        return;
      }
      const timeout = setTimeout(() => {
        pendingCreateTargetResponses.delete(targetId);
        clientSocket.write(
          encodeWebSocketText({
            id,
            error: {
              code: -32000,
              message:
                "Target.createTarget timed out waiting for Target.attachedToTarget",
            },
          }),
        );
      }, 5_000);
      pendingCreateTargetResponses.set(targetId, { id, timeout });
    },
  };
}

function writeCreateTargetResponse(clientSocket, id, targetId) {
  clientSocket.write(
    encodeWebSocketText({
      id,
      result: { targetId },
    }),
  );
}

function attachedTargetIdFromFrame(frame) {
  let message;
  try {
    message = JSON.parse(frame.payload.toString("utf8"));
  } catch {
    return null;
  }
  if (message?.method !== "Target.attachedToTarget") {
    return null;
  }
  return message.params?.targetInfo?.targetId ?? null;
}

function rewriteDevToolsHttpBody({ body, headers, address, port, backendPort }) {
  const contentType = String(headers["content-type"] ?? "");
  if (!contentType.includes("application/json")) {
    return body;
  }

  let parsed;
  try {
    parsed = JSON.parse(body.toString("utf8"));
  } catch {
    return body;
  }

  const rewritten = rewriteDevToolsWebSocketUrls(parsed, {
    address,
    port,
    backendPort,
  });
  return Buffer.from(JSON.stringify(rewritten), "utf8");
}

function rewriteDevToolsWebSocketUrls(value, { address, port, backendPort }) {
  if (Array.isArray(value)) {
    return value.map((item) =>
      rewriteDevToolsWebSocketUrls(item, { address, port, backendPort }),
    );
  }
  if (!value || typeof value !== "object") {
    return value;
  }

  const result = { ...value };
  if (typeof result.webSocketDebuggerUrl === "string") {
    result.webSocketDebuggerUrl = result.webSocketDebuggerUrl.replace(
      `ws://${address}:${backendPort}/`,
      `ws://${address}:${port}/`,
    );
  }
  for (const [key, nested] of Object.entries(result)) {
    if (key !== "webSocketDebuggerUrl") {
      result[key] = rewriteDevToolsWebSocketUrls(nested, {
        address,
        port,
        backendPort,
      });
    }
  }
  return result;
}

function rewriteRequestHeaders(headers, address, backendPort) {
  const next = {};
  for (const [name, value] of Object.entries(headers)) {
    if (name.toLowerCase() === "host") {
      next.host = `${address}:${backendPort}`;
      continue;
    }
    if (!HOP_BY_HOP_HEADERS.has(name.toLowerCase())) {
      next[name] = value;
    }
  }
  return next;
}

function rewriteResponseHeaders(headers, contentLength) {
  const next = {};
  for (const [name, value] of Object.entries(headers)) {
    const normalized = name.toLowerCase();
    if (HOP_BY_HOP_HEADERS.has(normalized) || normalized === "content-length") {
      continue;
    }
    next[name] = value;
  }
  next["content-length"] = String(contentLength);
  return next;
}

function buildBackendUpgradeRequest(request, address, backendPort) {
  const lines = [
    `${request.method} ${request.url} HTTP/${request.httpVersion}`,
    `Host: ${address}:${backendPort}`,
  ];
  for (const [name, value] of Object.entries(request.headers)) {
    if (name.toLowerCase() === "host") {
      continue;
    }
    if (Array.isArray(value)) {
      for (const item of value) {
        lines.push(`${name}: ${item}`);
      }
    } else if (value != null) {
      lines.push(`${name}: ${value}`);
    }
  }
  return `${lines.join("\r\n")}\r\n\r\n`;
}

function createFrameParser({ onFrame }) {
  let buffer = Buffer.alloc(0);
  return {
    push(chunk) {
      buffer = Buffer.concat([buffer, chunk]);
      while (buffer.length >= 2) {
        const parsed = parseWebSocketFrame(buffer);
        if (!parsed) {
          return;
        }
        buffer = buffer.subarray(parsed.frameLength);
        onFrame(parsed.frame);
      }
    },
  };
}

function parseWebSocketFrame(buffer) {
  const first = buffer[0];
  const second = buffer[1];
  const opcode = first & 0x0f;
  const masked = (second & 0x80) !== 0;
  let length = second & 0x7f;
  let offset = 2;

  if (length === 126) {
    if (buffer.length < offset + 2) {
      return null;
    }
    length = buffer.readUInt16BE(offset);
    offset += 2;
  } else if (length === 127) {
    if (buffer.length < offset + 8) {
      return null;
    }
    const high = buffer.readUInt32BE(offset);
    const low = buffer.readUInt32BE(offset + 4);
    offset += 8;
    if (high !== 0 || low > Number.MAX_SAFE_INTEGER) {
      throw new Error("WebSocket frame is too large");
    }
    length = low;
  }

  const maskOffset = offset;
  if (masked) {
    offset += 4;
  }
  const frameLength = offset + length;
  if (buffer.length < frameLength) {
    return null;
  }

  let payload = buffer.subarray(offset, frameLength);
  if (masked) {
    const mask = buffer.subarray(maskOffset, maskOffset + 4);
    payload = Buffer.from(payload);
    for (let index = 0; index < payload.length; index += 1) {
      payload[index] ^= mask[index % 4];
    }
  }

  return {
    frame: {
      fin: (first & 0x80) !== 0,
      opcode,
      payload,
    },
    frameLength,
  };
}

function encodeWebSocketText(message) {
  return encodeWebSocketFrame(Buffer.from(JSON.stringify(message), "utf8"), 0x1, false);
}

function encodeWebSocketFrame(payload, opcode, masked) {
  const payloadBuffer = Buffer.isBuffer(payload) ? payload : Buffer.from(payload);
  const length = payloadBuffer.length;
  const headerLength = length < 126 ? 2 : length <= 0xffff ? 4 : 10;
  const maskLength = masked ? 4 : 0;
  const frame = Buffer.alloc(headerLength + maskLength + length);
  frame[0] = 0x80 | opcode;
  let offset = 2;
  if (length < 126) {
    frame[1] = masked ? 0x80 | length : length;
  } else if (length <= 0xffff) {
    frame[1] = masked ? 0x80 | 126 : 126;
    frame.writeUInt16BE(length, offset);
    offset += 2;
  } else {
    frame[1] = masked ? 0x80 | 127 : 127;
    frame.writeUInt32BE(0, offset);
    frame.writeUInt32BE(length, offset + 4);
    offset += 8;
  }

  if (masked) {
    const mask = crypto.randomBytes(4);
    mask.copy(frame, offset);
    offset += 4;
    for (let index = 0; index < payloadBuffer.length; index += 1) {
      frame[offset + index] = payloadBuffer[index] ^ mask[index % 4];
    }
  } else {
    payloadBuffer.copy(frame, offset);
  }
  return frame;
}

function isBrowserWebSocketPath(path) {
  return typeof path === "string" && path.startsWith("/devtools/browser/");
}

module.exports = {
  encodeWebSocketFrame,
  parseWebSocketFrame,
  rewriteDevToolsWebSocketUrls,
  startRemoteDebuggingProxy,
};

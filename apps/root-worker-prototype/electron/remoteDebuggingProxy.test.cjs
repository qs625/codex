const test = require("node:test");
const assert = require("node:assert/strict");
const http = require("node:http");
const net = require("node:net");

const {
  encodeWebSocketFrame,
  parseWebSocketFrame,
  rewriteDevToolsWebSocketUrls,
  startRemoteDebuggingProxy,
} = require("./remoteDebuggingProxy.cjs");

test("rewriteDevToolsWebSocketUrls keeps CDP clients on the proxy port", () => {
  assert.deepEqual(
    rewriteDevToolsWebSocketUrls(
      {
        webSocketDebuggerUrl: "ws://127.0.0.1:9223/devtools/browser/abc",
        nested: [
          {
            webSocketDebuggerUrl: "ws://127.0.0.1:9223/devtools/page/one",
          },
        ],
      },
      { address: "127.0.0.1", port: "9222", backendPort: "9223" },
    ),
    {
      webSocketDebuggerUrl: "ws://127.0.0.1:9222/devtools/browser/abc",
      nested: [
        {
          webSocketDebuggerUrl: "ws://127.0.0.1:9222/devtools/page/one",
        },
      ],
    },
  );
});

test("remote debugging proxy rewrites /json/version WebSocket URLs", async (t) => {
  let backendPort = null;
  const backend = await startFakeBackend({
    requestHandler(_request, response) {
      response.writeHead(200, { "content-type": "application/json" });
      response.end(
        JSON.stringify({
          Browser: "Root Worker Runtime",
          webSocketDebuggerUrl:
            `ws://127.0.0.1:${backendPort}/devtools/browser/backend`,
        }),
      );
    },
  });
  backendPort = backend.port;
  t.after(() => backend.close());

  const proxyPort = await getFreePort();
  const proxy = startRemoteDebuggingProxy({
    address: "127.0.0.1",
    port: String(proxyPort),
    backendPort: String(backend.port),
    createTarget: async () => ({ targetId: "unused" }),
    logger: quietLogger(),
  });
  t.after(() => proxy.close());
  await waitForListening(proxy.server);

  const response = await fetchJson(`http://127.0.0.1:${proxyPort}/json/version`);

  assert.equal(
    response.webSocketDebuggerUrl,
    `ws://127.0.0.1:${proxyPort}/devtools/browser/backend`,
  );
});

test("remote debugging proxy maps Target.createTarget to Browser panel creation", async (t) => {
  const backendMessages = [];
  let backendSocket = null;
  const backend = await startFakeBackend({
    upgradeHandler(_request, socket) {
      backendSocket = socket;
      socket.write(
        "HTTP/1.1 101 Switching Protocols\r\n" +
          "Upgrade: websocket\r\n" +
          "Connection: Upgrade\r\n" +
          "\r\n",
      );
      let buffer = Buffer.alloc(0);
      socket.on("data", (chunk) => {
        buffer = Buffer.concat([buffer, chunk]);
        const parsed = parseWebSocketFrame(buffer);
        if (!parsed) {
          return;
        }
        buffer = buffer.subarray(parsed.frameLength);
        backendMessages.push(JSON.parse(parsed.frame.payload.toString("utf8")));
      });
    },
  });
  t.after(() => backend.close());

  const proxyPort = await getFreePort();
  const createdTargets = [];
  const proxy = startRemoteDebuggingProxy({
    address: "127.0.0.1",
    port: String(proxyPort),
    backendPort: String(backend.port),
    createTarget: async (url) => {
      createdTargets.push(url);
      setImmediate(() => {
        backendSocket.write(
          encodeWebSocketFrame(
            Buffer.from(
              JSON.stringify({
                method: "Target.attachedToTarget",
                params: {
                  sessionId: "session-1",
                  targetInfo: {
                    targetId: "browser-panel-target-1",
                    type: "page",
                    url: "about:blank",
                  },
                },
              }),
            ),
            0x1,
            false,
          ),
        );
      });
      return { targetId: "browser-panel-target-1" };
    },
    logger: quietLogger(),
  });
  t.after(() => proxy.close());
  await waitForListening(proxy.server);

  const socket = await connectWebSocket(proxyPort, "/devtools/browser/root");
  t.after(() => socket.destroy());

  const messages = readWebSocketMessages(socket, 2);
  socket.write(
    encodeWebSocketFrame(
      Buffer.from(
        JSON.stringify({
          id: 1,
          method: "Target.createTarget",
          params: { url: "about:blank" },
        }),
      ),
      0x1,
      true,
    ),
  );

  const [firstMessage, secondMessage] = await messages;
  assert.deepEqual(JSON.parse(firstMessage), {
    method: "Target.attachedToTarget",
    params: {
      sessionId: "session-1",
      targetInfo: {
        targetId: "browser-panel-target-1",
        type: "page",
        url: "about:blank",
      },
    },
  });
  assert.deepEqual(JSON.parse(secondMessage), {
    id: 1,
    result: { targetId: "browser-panel-target-1" },
  });
  assert.deepEqual(createdTargets, ["about:blank"]);
  assert.deepEqual(backendMessages, []);
});

test("remote debugging proxy forwards page initialization messages after createTarget", async (t) => {
  let backendSocket = null;
  const backendMessages = [];
  const backend = await startFakeBackend({
    upgradeHandler(_request, socket) {
      backendSocket = socket;
      socket.write(
        "HTTP/1.1 101 Switching Protocols\r\n" +
          "Upgrade: websocket\r\n" +
          "Connection: Upgrade\r\n" +
          "\r\n",
      );
      let buffer = Buffer.alloc(0);
      socket.on("data", (chunk) => {
        buffer = Buffer.concat([buffer, chunk]);
        while (true) {
          const parsed = parseWebSocketFrame(buffer);
          if (!parsed) {
            return;
          }
          buffer = buffer.subarray(parsed.frameLength);
          const message = JSON.parse(parsed.frame.payload.toString("utf8"));
          backendMessages.push(message);
          if (message.method === "Page.enable") {
            socket.write(
              encodeWebSocketFrame(
                Buffer.from(JSON.stringify({ id: message.id, result: {}, sessionId: message.sessionId })),
                0x1,
                false,
              ),
            );
          } else if (message.method === "Page.getFrameTree") {
            socket.write(
              encodeWebSocketFrame(
                Buffer.from(
                  JSON.stringify({
                    id: message.id,
                    result: {
                      frameTree: {
                        frame: {
                          id: "frame-1",
                          loaderId: "loader-1",
                          url: "about:blank",
                          mimeType: "text/html",
                          securityOrigin: "://",
                        },
                      },
                    },
                    sessionId: message.sessionId,
                  }),
                ),
                0x1,
                false,
              ),
            );
          }
        }
      });
    },
  });
  t.after(() => backend.close());

  const proxyPort = await getFreePort();
  const proxy = startRemoteDebuggingProxy({
    address: "127.0.0.1",
    port: String(proxyPort),
    backendPort: String(backend.port),
    createTarget: async () => {
      setImmediate(() => {
        backendSocket.write(
          encodeWebSocketFrame(
            Buffer.from(
              JSON.stringify({
                method: "Target.attachedToTarget",
                params: {
                  sessionId: "session-1",
                  targetInfo: {
                    targetId: "browser-panel-target-1",
                    type: "page",
                    url: "about:blank",
                  },
                },
              }),
            ),
            0x1,
            false,
          ),
        );
      });
      return { targetId: "browser-panel-target-1" };
    },
    logger: quietLogger(),
  });
  t.after(() => proxy.close());
  await waitForListening(proxy.server);

  const socket = await connectWebSocket(proxyPort, "/devtools/browser/root");
  t.after(() => socket.destroy());

  const createMessages = readWebSocketMessages(socket, 2);
  socket.write(
    encodeWebSocketFrame(
      Buffer.from(
        JSON.stringify({
          id: 1,
          method: "Target.createTarget",
          params: { url: "about:blank" },
        }),
      ),
      0x1,
      true,
    ),
  );
  await createMessages;

  const pageEnable = readWebSocketMessage(socket);
  socket.write(
    encodeWebSocketFrame(
      Buffer.from(
        JSON.stringify({
          id: 2,
          method: "Page.enable",
          params: {},
          sessionId: "session-1",
        }),
      ),
      0x1,
      true,
    ),
  );
  assert.deepEqual(JSON.parse(await pageEnable), {
    id: 2,
    result: {},
    sessionId: "session-1",
  });

  const frameTree = readWebSocketMessage(socket);
  socket.write(
    encodeWebSocketFrame(
      Buffer.from(
        JSON.stringify({
          id: 3,
          method: "Page.getFrameTree",
          params: {},
          sessionId: "session-1",
        }),
      ),
      0x1,
      true,
    ),
  );
  assert.deepEqual(JSON.parse(await frameTree), {
    id: 3,
    result: {
      frameTree: {
        frame: {
          id: "frame-1",
          loaderId: "loader-1",
          url: "about:blank",
          mimeType: "text/html",
          securityOrigin: "://",
        },
      },
    },
    sessionId: "session-1",
  });
  assert.deepEqual(
    backendMessages.map((message) => [message.method, message.sessionId]),
    [
      ["Page.enable", "session-1"],
      ["Page.getFrameTree", "session-1"],
    ],
  );
});

test("remote debugging proxy strips unsupported websocket extensions", async (t) => {
  let backendUpgradeHeaders = null;
  const backend = await startFakeBackend({
    upgradeHandler(request, socket) {
      backendUpgradeHeaders = request.headers;
      socket.write(
        "HTTP/1.1 101 Switching Protocols\r\n" +
          "Upgrade: websocket\r\n" +
          "Connection: Upgrade\r\n" +
          "Sec-WebSocket-Extensions: permessage-deflate\r\n" +
          "\r\n",
      );
    },
  });
  t.after(() => backend.close());

  const proxyPort = await getFreePort();
  const proxy = startRemoteDebuggingProxy({
    address: "127.0.0.1",
    port: String(proxyPort),
    backendPort: String(backend.port),
    createTarget: async () => ({ targetId: "unused" }),
    logger: quietLogger(),
  });
  t.after(() => proxy.close());
  await waitForListening(proxy.server);

  const { socket, handshake } = await connectWebSocket(
    proxyPort,
    "/devtools/page/page-target",
    {
      extensions: "permessage-deflate; client_max_window_bits",
      includeHandshake: true,
    },
  );
  t.after(() => socket.destroy());

  assert.equal(backendUpgradeHeaders["sec-websocket-extensions"], undefined);
  assert.doesNotMatch(handshake, /Sec-WebSocket-Extensions/i);
});

test("remote debugging proxy transparently forwards page target websocket messages", async (t) => {
  const backend = await startFakeBackend({
    upgradeHandler(_request, socket) {
      socket.write(
        "HTTP/1.1 101 Switching Protocols\r\n" +
          "Upgrade: websocket\r\n" +
          "Connection: Upgrade\r\n" +
          "\r\n",
      );
      let buffer = Buffer.alloc(0);
      socket.on("data", (chunk) => {
        buffer = Buffer.concat([buffer, chunk]);
        const parsed = parseWebSocketFrame(buffer);
        if (!parsed) {
          return;
        }
        buffer = buffer.subarray(parsed.frameLength);
        const message = JSON.parse(parsed.frame.payload.toString("utf8"));
        socket.write(
          encodeWebSocketFrame(
            Buffer.from(
              JSON.stringify({
                id: message.id,
                result: { echoedMethod: message.method },
              }),
            ),
            0x1,
            false,
          ),
        );
      });
    },
  });
  t.after(() => backend.close());

  const proxyPort = await getFreePort();
  const proxy = startRemoteDebuggingProxy({
    address: "127.0.0.1",
    port: String(proxyPort),
    backendPort: String(backend.port),
    createTarget: async () => {
      throw new Error("page websocket must not create Browser panel tabs");
    },
    logger: quietLogger(),
  });
  t.after(() => proxy.close());
  await waitForListening(proxy.server);

  const socket = await connectWebSocket(proxyPort, "/devtools/page/page-target");
  t.after(() => socket.destroy());

  const responseMessage = readWebSocketMessage(socket);
  socket.write(
    encodeWebSocketFrame(
      Buffer.from(
        JSON.stringify({
          id: 41,
          method: "Runtime.evaluate",
          params: { expression: "document.title" },
        }),
      ),
      0x1,
      true,
    ),
  );

  assert.deepEqual(JSON.parse(await responseMessage), {
    id: 41,
    result: { echoedMethod: "Runtime.evaluate" },
  });
});

test("remote debugging proxy rejects unsafe Target.createTarget URLs", async (t) => {
  const backend = await startFakeBackend({
    upgradeHandler(_request, socket) {
      socket.write(
        "HTTP/1.1 101 Switching Protocols\r\n" +
          "Upgrade: websocket\r\n" +
          "Connection: Upgrade\r\n" +
          "\r\n",
      );
    },
  });
  t.after(() => backend.close());

  const proxyPort = await getFreePort();
  const proxy = startRemoteDebuggingProxy({
    address: "127.0.0.1",
    port: String(proxyPort),
    backendPort: String(backend.port),
    createTarget: async () => {
      throw new Error("Only http and https URLs can open here.");
    },
    logger: quietLogger(),
  });
  t.after(() => proxy.close());
  await waitForListening(proxy.server);

  const socket = await connectWebSocket(proxyPort, "/devtools/browser/root");
  t.after(() => socket.destroy());

  const errorMessage = readWebSocketMessage(socket);
  socket.write(
    encodeWebSocketFrame(
      Buffer.from(
        JSON.stringify({
          id: 7,
          method: "Target.createTarget",
          params: { url: "javascript:alert(1)" },
        }),
      ),
      0x1,
      true,
    ),
  );

  const response = JSON.parse(await errorMessage);
  assert.equal(response.id, 7);
  assert.equal(response.error.code, -32000);
  assert.match(response.error.message, /Only http and https/);
});

function startFakeBackend({ requestHandler, upgradeHandler }) {
  const sockets = new Set();
  const server = http.createServer(
    requestHandler ??
      ((_request, response) => {
        response.writeHead(404);
        response.end();
      }),
  );
  if (upgradeHandler) {
    server.on("upgrade", upgradeHandler);
  }
  server.on("connection", (socket) => {
    sockets.add(socket);
    socket.on("close", () => sockets.delete(socket));
  });
  return new Promise((resolve) => {
    server.listen(0, "127.0.0.1", () => {
      resolve({
        close: () => {
          for (const socket of sockets) {
            socket.destroy();
          }
          return new Promise((done) => server.close(done));
        },
        port: server.address().port,
      });
    });
  });
}

function getFreePort() {
  return new Promise((resolve) => {
    const server = net.createServer();
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      server.close(() => resolve(port));
    });
  });
}

function waitForListening(server) {
  return new Promise((resolve) => {
    if (server.listening) {
      resolve();
      return;
    }
    server.on("listening", resolve);
  });
}

function fetchJson(url) {
  return new Promise((resolve, reject) => {
    http
      .get(url, (response) => {
        const chunks = [];
        response.on("data", (chunk) => chunks.push(chunk));
        response.on("end", () => {
          try {
            resolve(JSON.parse(Buffer.concat(chunks).toString("utf8")));
          } catch (error) {
            reject(error);
          }
        });
      })
      .on("error", reject);
  });
}

function connectWebSocket(port, path, options = {}) {
  return new Promise((resolve, reject) => {
    const socket = net.connect(port, "127.0.0.1");
    let buffer = Buffer.alloc(0);
    socket.on("connect", () => {
      const extensionHeader = options.extensions
        ? `Sec-WebSocket-Extensions: ${options.extensions}\r\n`
        : "";
      socket.write(
        `GET ${path} HTTP/1.1\r\n` +
          `Host: 127.0.0.1:${port}\r\n` +
          "Upgrade: websocket\r\n" +
          "Connection: Upgrade\r\n" +
          "Sec-WebSocket-Key: test-key\r\n" +
          "Sec-WebSocket-Version: 13\r\n" +
          extensionHeader +
          "\r\n",
      );
    });
    socket.on("data", function onData(chunk) {
      buffer = Buffer.concat([buffer, chunk]);
      const headerEnd = buffer.indexOf("\r\n\r\n");
      if (headerEnd === -1) {
        return;
      }
      socket.removeListener("data", onData);
      const handshake = buffer.subarray(0, headerEnd + 4).toString("latin1");
      resolve(options.includeHandshake ? { socket, handshake } : socket);
    });
    socket.on("error", reject);
  });
}

function readWebSocketMessage(socket) {
  return new Promise((resolve) => {
    let buffer = Buffer.alloc(0);
    socket.on("data", function onData(chunk) {
      buffer = Buffer.concat([buffer, chunk]);
      const parsed = parseWebSocketFrame(buffer);
      if (!parsed) {
        return;
      }
      socket.removeListener("data", onData);
      resolve(parsed.frame.payload.toString("utf8"));
    });
  });
}

function readWebSocketMessages(socket, count) {
  return new Promise((resolve) => {
    let buffer = Buffer.alloc(0);
    const messages = [];
    socket.on("data", function onData(chunk) {
      buffer = Buffer.concat([buffer, chunk]);
      while (messages.length < count) {
        const parsed = parseWebSocketFrame(buffer);
        if (!parsed) {
          return;
        }
        buffer = buffer.subarray(parsed.frameLength);
        messages.push(parsed.frame.payload.toString("utf8"));
      }
      socket.removeListener("data", onData);
      resolve(messages);
    });
  });
}

function quietLogger() {
  return {
    error() {},
    warn() {},
  };
}

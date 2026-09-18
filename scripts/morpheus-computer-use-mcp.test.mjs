import assert from "node:assert/strict";
import test from "node:test";
import { PassThrough } from "node:stream";
import { createRequire } from "node:module";

import {
  createComputerUseMcpServer,
  runComputerUseMcpServer,
} from "./morpheus-computer-use-mcp.mjs";

const require = createRequire(import.meta.url);
const {
  createComputerUseManager,
} = require("../apps/root-worker-prototype/electron/computerUse.cjs");

function fakeNativeClient(options = {}) {
  let observeCount = 0;
  const actPayloads = [];
  return {
    actPayloads,
    async observe(payload = {}) {
      observeCount += 1;
      const accessibilityTrusted =
        options.accessibilityTrustedSequence?.[observeCount - 1] ??
        options.accessibilityTrusted ??
        true;
      return {
        cursor: { x: 100 + observeCount, y: 200 + observeCount },
        systemCursor: { x: 100 + observeCount, y: 200 + observeCount },
        frontmostApp: {
          name: options.activeAppName ?? "Finder",
          bundleIdentifier: options.bundleIdentifier ?? "com.apple.finder",
          processIdentifier: 42,
          frontmost: true,
          window: {
            title: `Window ${observeCount}`,
            position: { x: 10, y: 20 },
            size: { width: 640, height: 480 },
          },
        },
        targetApp: {
          name: options.activeAppName ?? "Finder",
          bundleIdentifier: options.bundleIdentifier ?? "com.apple.finder",
          processIdentifier: 42,
          frontmost: true,
          window: {
            title: `Window ${observeCount}`,
            position: { x: 10, y: 20 },
            size: { width: 640, height: 480 },
          },
        },
        targetVisibility: "frontmost",
        accessibilityTrusted,
        screenshot: {
          path: `/tmp/screen-${observeCount}.png`,
          mimeType: "image/png",
          byteSize: 10,
          dataUrl: "data:image/png;base64,AAAA",
        },
        perception: {
          accessibilityElements: [
            {
              role: "AXButton",
              title: "Continue",
              bounds: { x: 20, y: 40, width: 100, height: 30 },
              center: { x: 70, y: 55 },
              confidence: 0.85,
              source: "test",
            },
          ],
          limitations: [],
        },
      };
    },
    async act(payload) {
      actPayloads.push(payload);
      return { ok: true, method: payload.type };
    },
    async activateTarget() {
      return { activated: true };
    },
    async cleanup() {},
  };
}

function managerFactoryWithNative(nativeClient) {
  return async (options = {}) =>
    createComputerUseManager({
      nativeClient,
      includePerception: options.includePerception,
      perceptionLimit: options.perceptionLimit,
      safety: {
        operationBoundary: "computer-use-mcp-session",
        confirmRisk: options.confirmRisk,
        planOnly: options.planOnly,
      },
    });
}

test("computer use MCP lists typed tools", () => {
  const server = createComputerUseMcpServer({
    managerFactory: managerFactoryWithNative(fakeNativeClient()),
  });
  const names = server.listTools().map((tool) => tool.name);
  assert.deepEqual(names, [
    "computer.start_session",
    "computer.observe",
    "computer.find_text",
    "computer.act",
    "computer.stop",
    "computer.permissions_status",
  ]);
  const act = server.listTools().find((tool) => tool.name === "computer.act");
  assert.deepEqual(act.inputSchema.required, ["action"]);
  assert.equal(act.inputSchema.properties.action.type, "object");
});

test("computer use MCP observe returns typed evidence without screenshot data by default", async () => {
  const server = createComputerUseMcpServer({
    managerFactory: managerFactoryWithNative(fakeNativeClient()),
  });
  const started = await server.callTool("computer.start_session", {
    app: "com.apple.finder",
  });
  assert.equal(started.isError, false);
  assert.equal(started.structuredContent.sessionId, "default");
  assert.equal(started.structuredContent.permissions.screenRecording, "granted");
  assert.equal(started.structuredContent.permissions.accessibilityTrusted, true);
  assert.equal(
    started.structuredContent.state.observation.screenshot.dataUrl,
    undefined,
  );
  assert.equal(
    started.structuredContent.state.observation.screenshot.dataUrlOmitted,
    true,
  );
  assert.match(started.content[0].text, /computer.start_session/);
  await server.close();
});

test("computer use MCP blocks find_text when Accessibility is not trusted", async () => {
  const nativeClient = fakeNativeClient({ accessibilityTrusted: false });
  const server = createComputerUseMcpServer({
    managerFactory: managerFactoryWithNative(nativeClient),
  });
  await server.callTool("computer.start_session", {});
  const result = await server.callTool("computer.find_text", { text: "Continue" });
  assert.equal(result.isError, false);
  assert.equal(result.structuredContent.status, "blocked");
  assert.equal(result.structuredContent.policy.kind, "needs-permission");
  assert.match(result.structuredContent.policy.reason, /Accessibility permission/);
  assert.equal(nativeClient.actPayloads.length, 0);
  assert.equal(result.structuredContent.audit.completion.status, "blocked");
  await server.close();
});

test("computer use MCP side effects preserve policy audit and native evidence", async () => {
  const nativeClient = fakeNativeClient();
  const server = createComputerUseMcpServer({
    managerFactory: managerFactoryWithNative(nativeClient),
  });
  await server.callTool("computer.start_session", {});
  const result = await server.callTool("computer.act", {
    action: { type: "click", x: 70, y: 55 },
  });
  assert.equal(result.structuredContent.status, "completed");
  assert.equal(result.structuredContent.policy.kind, "side-effect");
  assert.equal(result.structuredContent.evidence.method, "click");
  assert.equal(
    result.structuredContent.audit.operationBoundary,
    "computer-use-mcp-session",
  );
  assert.equal(nativeClient.actPayloads.length, 1);
  await server.close();
});

test("computer use MCP permissions_status reports helper subject diagnostics", async () => {
  const server = createComputerUseMcpServer({
    managerFactory: managerFactoryWithNative(fakeNativeClient()),
    diagnosticsFactory: () => ({
      contract: "helper-as-mcp-server",
      mcpServer: {
        entrypoint: "/repo/scripts/morpheus-computer-use-mcp.mjs",
        execPath: "/usr/local/bin/node",
        cwd: "/repo",
      },
      permissionSubject: {
        bundleIdentifier: "com.openai.root-worker-prototype.runtime.dev",
        bundlePath: "/Applications/Root Worker Runtime.app",
        executablePath: "/usr/local/bin/node",
      },
    }),
  });
  const result = await server.callTool("computer.permissions_status", {});
  assert.equal(result.structuredContent.permissions.screenRecording, "granted");
  assert.equal(
    result.structuredContent.diagnostics.permissionSubject.bundleIdentifier,
    "com.openai.root-worker-prototype.runtime.dev",
  );
  assert.match(
    result.structuredContent.limitations[0].message,
    /Launcher supervises Runtime Capsule/,
  );
  await server.close();
});

test("computer use MCP permissions_status reports packaged helper app identity", async () => {
  const previous = {
    mode: process.env.MORPHEUS_COMPUTER_USE_HELPER_MODE,
    bundleId: process.env.MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_ID,
    bundlePath: process.env.MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_PATH,
    executable: process.env.MORPHEUS_COMPUTER_USE_HELPER_EXECUTABLE,
  };
  process.env.MORPHEUS_COMPUTER_USE_HELPER_MODE = "packaged-helper-app";
  process.env.MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_ID =
    "com.openai.root-worker-prototype.computer-use.dev";
  process.env.MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_PATH =
    "/Applications/Morpheus.app/Contents/Resources/computer-use-helper/Root Worker Computer Use.app";
  process.env.MORPHEUS_COMPUTER_USE_HELPER_EXECUTABLE =
    "/Applications/Morpheus.app/Contents/Resources/computer-use-helper/Root Worker Computer Use.app/Contents/MacOS/Root Worker Computer Use";
  try {
    const server = createComputerUseMcpServer({
      managerFactory: managerFactoryWithNative(fakeNativeClient()),
    });
    const result = await server.callTool("computer.permissions_status", {
      includeObservation: false,
    });
    assert.equal(
      result.structuredContent.diagnostics.helperMode,
      "packaged-helper-app",
    );
    assert.equal(
      result.structuredContent.diagnostics.permissionSubject.bundleIdentifier,
      "com.openai.root-worker-prototype.computer-use.dev",
    );
    assert.equal(
      result.structuredContent.diagnostics.permissionSubject.packagedHelperBundle,
      true,
    );
    assert.equal(
      result.structuredContent.diagnostics.permissionSubject.stablePermissionSubject,
      false,
    );
    assert.equal(
      result.structuredContent.diagnostics.permissionSubject.nativeControlSubject,
      "delegated-swift-script-and-screencapture",
    );
    assert.match(
      result.structuredContent.diagnostics.permissionSubject.executablePath,
      /Root Worker Computer Use$/,
    );
    assert.ok(
      result.structuredContent.limitations.some(
        (limitation) =>
          limitation.code === "native-permission-subject-not-contained" &&
          /delegates to Swift and screencapture/.test(limitation.message),
      ),
    );
    await server.close();
  } finally {
    restoreEnv("MORPHEUS_COMPUTER_USE_HELPER_MODE", previous.mode);
    restoreEnv("MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_ID", previous.bundleId);
    restoreEnv("MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_PATH", previous.bundlePath);
    restoreEnv("MORPHEUS_COMPUTER_USE_HELPER_EXECUTABLE", previous.executable);
  }
});

test("computer use MCP stdio handles initialize, tools/list, and tools/call", async () => {
  const input = new PassThrough();
  const output = new PassThrough();
  const server = createComputerUseMcpServer({
    managerFactory: managerFactoryWithNative(fakeNativeClient()),
  });
  const done = runComputerUseMcpServer({ input, output, server });
  const lines = [];
  output.setEncoding("utf8");
  output.on("data", (chunk) => {
    lines.push(...chunk.split("\n").filter(Boolean));
  });

  input.write(
    `${JSON.stringify({ jsonrpc: "2.0", id: 1, method: "initialize", params: {} })}\n`,
  );
  input.write(
    `${JSON.stringify({ jsonrpc: "2.0", id: 2, method: "tools/list", params: {} })}\n`,
  );
  input.write(
    `${JSON.stringify({
      jsonrpc: "2.0",
      id: 3,
      method: "tools/call",
      params: { name: "computer.observe", arguments: {} },
    })}\n`,
  );

  await waitFor(() => lines.length >= 3);
  input.end();
  await done;

  const responses = lines.map((line) => JSON.parse(line));
  assert.equal(responses[0].result.serverInfo.name, "morpheus-computer-use");
  assert.equal(responses[1].result.tools.length, 6);
  assert.equal(responses[2].result.structuredContent.tool, "computer.observe");
});

async function waitFor(predicate) {
  const started = Date.now();
  while (!predicate()) {
    if (Date.now() - started > 1000) {
      throw new Error("timed out waiting for condition");
    }
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
}

function restoreEnv(name, value) {
  if (value === undefined) {
    delete process.env[name];
  } else {
    process.env[name] = value;
  }
}

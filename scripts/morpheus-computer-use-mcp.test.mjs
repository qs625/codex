import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
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
    nativeExecutable:
      process.env.MORPHEUS_COMPUTER_USE_NATIVE_HELPER_EXECUTABLE,
    stableHelperAppPath:
      process.env.MORPHEUS_COMPUTER_USE_STABLE_HELPER_APP_PATH,
  };
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "computer-use-helper-"));
  const helperBundlePath = path.join(root, "Root Worker Computer Use.app");
  const helperExecutable = path.join(
    helperBundlePath,
    "Contents",
    "MacOS",
    "Root Worker Computer Use",
  );
  const nativeHelperExecutable = path.join(
    helperBundlePath,
    "Contents",
    "MacOS",
    "morpheus-computer-use-native",
  );
  fs.mkdirSync(path.dirname(nativeHelperExecutable), { recursive: true });
  fs.writeFileSync(helperExecutable, "#!/bin/sh\n", { mode: 0o755 });
  fs.writeFileSync(nativeHelperExecutable, "native", { mode: 0o755 });
  process.env.MORPHEUS_COMPUTER_USE_HELPER_MODE = "packaged-helper-app";
  process.env.MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_ID =
    "com.openai.root-worker-prototype.computer-use.dev";
  process.env.MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_PATH = helperBundlePath;
  process.env.MORPHEUS_COMPUTER_USE_HELPER_EXECUTABLE = helperExecutable;
  process.env.MORPHEUS_COMPUTER_USE_NATIVE_HELPER_EXECUTABLE =
    nativeHelperExecutable;
  process.env.MORPHEUS_COMPUTER_USE_STABLE_HELPER_APP_PATH = helperBundlePath;
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
      true,
    );
    assert.equal(
      result.structuredContent.diagnostics.permissionSubject.nativeControlSubject,
      "stable-packaged-native-helper-executable",
    );
    assert.equal(
      result.structuredContent.diagnostics.permissionSubject.executablePath,
      nativeHelperExecutable,
    );
    assert.equal(
      result.structuredContent.diagnostics.permissionSubject.helperExecutablePath,
      helperExecutable,
    );
    assert.equal(
      result.structuredContent.diagnostics.permissionSubject.nativeHelperExecutablePath,
      nativeHelperExecutable,
    );
    assert.ok(
      !result.structuredContent.limitations.some(
        (limitation) =>
          limitation.code === "native-permission-subject-not-contained",
      ),
    );
    await server.close();
  } finally {
    restoreEnv("MORPHEUS_COMPUTER_USE_HELPER_MODE", previous.mode);
    restoreEnv("MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_ID", previous.bundleId);
    restoreEnv("MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_PATH", previous.bundlePath);
    restoreEnv("MORPHEUS_COMPUTER_USE_HELPER_EXECUTABLE", previous.executable);
    restoreEnv(
      "MORPHEUS_COMPUTER_USE_NATIVE_HELPER_EXECUTABLE",
      previous.nativeExecutable,
    );
    restoreEnv(
      "MORPHEUS_COMPUTER_USE_STABLE_HELPER_APP_PATH",
      previous.stableHelperAppPath,
    );
    fs.rmSync(root, { force: true, recursive: true });
  }
});

test("computer use MCP permissions_status keeps delegated limitation without native helper executable", async () => {
  const previous = {
    mode: process.env.MORPHEUS_COMPUTER_USE_HELPER_MODE,
    bundleId: process.env.MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_ID,
    bundlePath: process.env.MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_PATH,
    executable: process.env.MORPHEUS_COMPUTER_USE_HELPER_EXECUTABLE,
    nativeExecutable:
      process.env.MORPHEUS_COMPUTER_USE_NATIVE_HELPER_EXECUTABLE,
    stableHelperAppPath:
      process.env.MORPHEUS_COMPUTER_USE_STABLE_HELPER_APP_PATH,
  };
  process.env.MORPHEUS_COMPUTER_USE_HELPER_MODE = "packaged-helper-app";
  process.env.MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_ID =
    "com.openai.root-worker-prototype.computer-use.dev";
  process.env.MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_PATH =
    "/Applications/Morpheus.app/Contents/Resources/computer-use-helper/Root Worker Computer Use.app";
  process.env.MORPHEUS_COMPUTER_USE_HELPER_EXECUTABLE =
    "/Applications/Morpheus.app/Contents/Resources/computer-use-helper/Root Worker Computer Use.app/Contents/MacOS/Root Worker Computer Use";
  process.env.MORPHEUS_COMPUTER_USE_NATIVE_HELPER_EXECUTABLE =
    "/missing/morpheus-computer-use-native";
  try {
    const server = createComputerUseMcpServer({
      managerFactory: managerFactoryWithNative(fakeNativeClient()),
    });
    const result = await server.callTool("computer.permissions_status", {
      includeObservation: false,
    });
    assert.equal(
      result.structuredContent.diagnostics.permissionSubject.stablePermissionSubject,
      false,
    );
    assert.equal(
      result.structuredContent.diagnostics.permissionSubject.nativeControlSubject,
      "delegated-swift-script-and-screencapture",
    );
    assert.ok(
      result.structuredContent.limitations.some(
        (limitation) =>
          limitation.code === "native-permission-subject-not-contained" &&
          /no packaged native helper executable/.test(limitation.message),
      ),
    );
    await server.close();
  } finally {
    restoreEnv("MORPHEUS_COMPUTER_USE_HELPER_MODE", previous.mode);
    restoreEnv("MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_ID", previous.bundleId);
    restoreEnv("MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_PATH", previous.bundlePath);
    restoreEnv("MORPHEUS_COMPUTER_USE_HELPER_EXECUTABLE", previous.executable);
    restoreEnv(
      "MORPHEUS_COMPUTER_USE_NATIVE_HELPER_EXECUTABLE",
      previous.nativeExecutable,
    );
    restoreEnv(
      "MORPHEUS_COMPUTER_USE_STABLE_HELPER_APP_PATH",
      previous.stableHelperAppPath,
    );
  }
});

test("computer use MCP permissions_status reports release-local packaged helper limitation", async () => {
  const previous = {
    mode: process.env.MORPHEUS_COMPUTER_USE_HELPER_MODE,
    bundleId: process.env.MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_ID,
    bundlePath: process.env.MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_PATH,
    executable: process.env.MORPHEUS_COMPUTER_USE_HELPER_EXECUTABLE,
    nativeExecutable:
      process.env.MORPHEUS_COMPUTER_USE_NATIVE_HELPER_EXECUTABLE,
    stableHelperAppPath:
      process.env.MORPHEUS_COMPUTER_USE_STABLE_HELPER_APP_PATH,
  };
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "computer-use-helper-"));
  const helperBundlePath = path.join(
    root,
    "release-local",
    "Root Worker Computer Use.app",
  );
  const helperExecutable = path.join(
    helperBundlePath,
    "Contents",
    "MacOS",
    "Root Worker Computer Use",
  );
  const nativeHelperExecutable = path.join(
    helperBundlePath,
    "Contents",
    "MacOS",
    "morpheus-computer-use-native",
  );
  fs.mkdirSync(path.dirname(nativeHelperExecutable), { recursive: true });
  fs.writeFileSync(helperExecutable, "#!/bin/sh\n", { mode: 0o755 });
  fs.writeFileSync(nativeHelperExecutable, "native", { mode: 0o755 });
  process.env.MORPHEUS_COMPUTER_USE_HELPER_MODE = "packaged-helper-app";
  process.env.MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_ID =
    "com.openai.root-worker-prototype.computer-use.dev";
  process.env.MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_PATH = helperBundlePath;
  process.env.MORPHEUS_COMPUTER_USE_HELPER_EXECUTABLE = helperExecutable;
  process.env.MORPHEUS_COMPUTER_USE_NATIVE_HELPER_EXECUTABLE =
    nativeHelperExecutable;
  process.env.MORPHEUS_COMPUTER_USE_STABLE_HELPER_APP_PATH = path.join(
    root,
    "Applications",
    "Root Worker Computer Use.app",
  );
  try {
    const server = createComputerUseMcpServer({
      managerFactory: managerFactoryWithNative(fakeNativeClient()),
    });
    const result = await server.callTool("computer.permissions_status", {
      includeObservation: false,
    });
    assert.equal(
      result.structuredContent.diagnostics.permissionSubject
        .packagedNativeHelperExecutable,
      true,
    );
    assert.equal(
      result.structuredContent.diagnostics.permissionSubject.usesStableHelperApp,
      false,
    );
    assert.equal(
      result.structuredContent.diagnostics.permissionSubject.stablePermissionSubject,
      false,
    );
    assert.equal(
      result.structuredContent.diagnostics.permissionSubject.nativeControlSubject,
      "release-local-packaged-native-helper-executable",
    );
    assert.ok(
      result.structuredContent.limitations.some(
        (limitation) =>
          limitation.code === "native-permission-subject-not-contained" &&
          /release-local/.test(limitation.message),
      ),
    );
    await server.close();
  } finally {
    restoreEnv("MORPHEUS_COMPUTER_USE_HELPER_MODE", previous.mode);
    restoreEnv("MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_ID", previous.bundleId);
    restoreEnv("MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_PATH", previous.bundlePath);
    restoreEnv("MORPHEUS_COMPUTER_USE_HELPER_EXECUTABLE", previous.executable);
    restoreEnv(
      "MORPHEUS_COMPUTER_USE_NATIVE_HELPER_EXECUTABLE",
      previous.nativeExecutable,
    );
    restoreEnv(
      "MORPHEUS_COMPUTER_USE_STABLE_HELPER_APP_PATH",
      previous.stableHelperAppPath,
    );
    fs.rmSync(root, { force: true, recursive: true });
  }
});

test("computer use MCP permissions_status resolves native helper executable from bundle", async () => {
  const previous = {
    mode: process.env.MORPHEUS_COMPUTER_USE_HELPER_MODE,
    bundleId: process.env.MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_ID,
    bundlePath: process.env.MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_PATH,
    executable: process.env.MORPHEUS_COMPUTER_USE_HELPER_EXECUTABLE,
    nativeExecutable:
      process.env.MORPHEUS_COMPUTER_USE_NATIVE_HELPER_EXECUTABLE,
    stableHelperAppPath:
      process.env.MORPHEUS_COMPUTER_USE_STABLE_HELPER_APP_PATH,
  };
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "computer-use-helper-"));
  const helperBundlePath = path.join(root, "Root Worker Computer Use.app");
  const helperExecutable = path.join(
    helperBundlePath,
    "Contents",
    "MacOS",
    "Root Worker Computer Use",
  );
  const nativeHelperExecutable = path.join(
    helperBundlePath,
    "Contents",
    "MacOS",
    "morpheus-computer-use-native",
  );
  fs.mkdirSync(path.dirname(nativeHelperExecutable), { recursive: true });
  fs.writeFileSync(helperExecutable, "#!/bin/sh\n", { mode: 0o755 });
  fs.writeFileSync(nativeHelperExecutable, "native", { mode: 0o755 });
  process.env.MORPHEUS_COMPUTER_USE_HELPER_MODE = "packaged-helper-app";
  process.env.MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_ID =
    "com.openai.root-worker-prototype.computer-use.dev";
  process.env.MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_PATH = helperBundlePath;
  process.env.MORPHEUS_COMPUTER_USE_HELPER_EXECUTABLE = helperExecutable;
  process.env.MORPHEUS_COMPUTER_USE_STABLE_HELPER_APP_PATH = helperBundlePath;
  delete process.env.MORPHEUS_COMPUTER_USE_NATIVE_HELPER_EXECUTABLE;
  try {
    const server = createComputerUseMcpServer({
      managerFactory: managerFactoryWithNative(fakeNativeClient()),
    });
    const result = await server.callTool("computer.permissions_status", {
      includeObservation: false,
    });
    assert.equal(
      result.structuredContent.diagnostics.permissionSubject.stablePermissionSubject,
      true,
    );
    assert.equal(
      result.structuredContent.diagnostics.permissionSubject.executablePath,
      nativeHelperExecutable,
    );
    assert.equal(
      result.structuredContent.diagnostics.permissionSubject.nativeControlSubject,
      "stable-packaged-native-helper-executable",
    );
    await server.close();
  } finally {
    restoreEnv("MORPHEUS_COMPUTER_USE_HELPER_MODE", previous.mode);
    restoreEnv("MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_ID", previous.bundleId);
    restoreEnv("MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_PATH", previous.bundlePath);
    restoreEnv("MORPHEUS_COMPUTER_USE_HELPER_EXECUTABLE", previous.executable);
    restoreEnv(
      "MORPHEUS_COMPUTER_USE_NATIVE_HELPER_EXECUTABLE",
      previous.nativeExecutable,
    );
    restoreEnv(
      "MORPHEUS_COMPUTER_USE_STABLE_HELPER_APP_PATH",
      previous.stableHelperAppPath,
    );
    fs.rmSync(root, { force: true, recursive: true });
  }
});

test("computer use MCP stdio defaults to RMCP line JSON initialize, tools/list, and tools/call", async () => {
  const input = new PassThrough();
  const output = new PassThrough();
  const server = createComputerUseMcpServer({
    managerFactory: managerFactoryWithNative(fakeNativeClient()),
    diagnosticsFactory: () => ({
      contract: "helper-as-mcp-server",
      mcpServer: { entrypoint: "/helper/server.mjs" },
      permissionSubject: {
        bundleIdentifier: "com.openai.root-worker.computer-use",
        bundlePath: "/Applications/Morpheus.app/Contents/Resources/computer-use-helper/Root Worker Computer Use.app",
        executablePath:
          "/Applications/Morpheus.app/Contents/Resources/computer-use-helper/Root Worker Computer Use.app/Contents/MacOS/morpheus-computer-use-native",
        packagedHelperBundle: true,
        packagedNativeHelperExecutable: true,
        stablePermissionSubject: true,
        nativeControlSubject: "packaged-native-helper-executable",
      },
    }),
  });
  const done = runComputerUseMcpServer({ input, output, server });
  const lines = collectLineJsonMessages(output);

  writeLineJson(input, { jsonrpc: "2.0", id: 1, method: "initialize", params: {} });
  writeLineJson(input, { jsonrpc: "2.0", id: 2, method: "tools/list", params: {} });
  writeLineJson(input, {
    jsonrpc: "2.0",
    id: 3,
    method: "tools/call",
    params: {
      name: "computer.permissions_status",
      arguments: { includeObservation: false },
    },
  });

  await waitFor(() => lines.length >= 3);
  input.end();
  await done;

  const responses = lines.map((line) => JSON.parse(line));
  assert.equal(responses[0].result.serverInfo.name, "morpheus-computer-use");
  assert.equal(responses[1].result.tools.length, 6);
  assert.equal(
    responses[2].result.structuredContent.diagnostics.permissionSubject
      .packagedNativeHelperExecutable,
    true,
  );
  assert.equal(
    responses[2].result.structuredContent.diagnostics.permissionSubject
      .stablePermissionSubject,
    true,
  );
  assert.equal(
    responses[2].result.structuredContent.diagnostics.permissionSubject
      .nativeControlSubject,
    "packaged-native-helper-executable",
  );
});

test("computer use MCP stdio keeps explicit content-length compatibility mode", async () => {
  await runFramedCompatibilityMode("content-length");
  await runFramedCompatibilityMode("framed");
});

async function runFramedCompatibilityMode(transportMode) {
  const input = new PassThrough();
  const output = new PassThrough();
  const server = createComputerUseMcpServer({
    managerFactory: managerFactoryWithNative(fakeNativeClient()),
  });
  const done = runComputerUseMcpServer({
    input,
    output,
    server,
    transportMode,
  });
  const responses = collectFramedMessages(output);

  writeFrame(input, {
    jsonrpc: "2.0",
    id: 1,
    method: "initialize",
    params: {},
  });
  writeFrame(input, {
    jsonrpc: "2.0",
    id: 2,
    method: "tools/list",
    params: {},
  });
  writeFrame(input, {
    jsonrpc: "2.0",
    id: 3,
    method: "tools/call",
    params: { name: "computer.observe", arguments: {} },
  });

  await waitFor(() => responses.length >= 3);
  input.end();
  await done;

  assert.equal(responses[0].result.serverInfo.name, "morpheus-computer-use");
  assert.equal(responses[1].result.tools.length, 6);
  assert.equal(responses[2].result.structuredContent.tool, "computer.observe");
}

function writeLineJson(input, message) {
  input.write(`${JSON.stringify(message)}\n`);
}

function collectLineJsonMessages(output) {
  const lines = [];
  output.setEncoding("utf8");
  output.on("data", (chunk) => {
    lines.push(...chunk.split("\n").filter(Boolean));
  });
  return lines;
}

function writeFrame(input, message) {
  const body = JSON.stringify(message);
  input.write(
    `Content-Length: ${Buffer.byteLength(body, "utf8")}\r\n\r\n${body}`,
  );
}

function collectFramedMessages(output) {
  const messages = [];
  let buffer = Buffer.alloc(0);
  output.on("data", (chunk) => {
    buffer = Buffer.concat([
      buffer,
      Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk),
    ]);
    while (true) {
      const headerEnd = buffer.indexOf("\r\n\r\n");
      if (headerEnd === -1) {
        return;
      }
      const header = buffer.subarray(0, headerEnd).toString("ascii");
      const lengthMatch = /^Content-Length:\s*(\d+)\s*$/im.exec(header);
      assert.ok(lengthMatch, `missing Content-Length header: ${header}`);
      const bodyStart = headerEnd + 4;
      const bodyEnd = bodyStart + Number(lengthMatch[1]);
      if (buffer.length < bodyEnd) {
        return;
      }
      const body = buffer.subarray(bodyStart, bodyEnd).toString("utf8");
      buffer = buffer.subarray(bodyEnd);
      messages.push(JSON.parse(body));
    }
  });
  return messages;
}

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

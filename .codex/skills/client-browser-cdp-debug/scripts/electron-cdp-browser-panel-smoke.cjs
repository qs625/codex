#!/usr/bin/env node
"use strict";

const { createServer } = require("node:http");
const { createRequire } = require("node:module");
const { spawn } = require("node:child_process");
const path = require("node:path");

const smokeTimeoutMs = parsePositiveIntegerEnv(
  "ROOT_WORKER_CDP_SMOKE_TIMEOUT_MS",
  90_000,
);

async function main() {
  const controller = new AbortController();
  const timeout = setTimeout(() => {
    const error = new Error(`Timed out after ${smokeTimeoutMs}ms`);
    error.exitCode = 124;
    controller.abort(error);
  }, smokeTimeoutMs);
  try {
    await runSmoke(controller.signal);
  } finally {
    clearTimeout(timeout);
  }
}

async function runSmoke(signal) {
  const appDir = requiredEnv("APP_DIR");
  const devUrl = requiredEnv("ROOT_WORKER_DEV_SERVER_URL");
  const cdpPort = requiredEnv("ROOT_WORKER_REMOTE_DEBUGGING_PORT");
  const screenshotPath =
    process.env.ROOT_WORKER_CDP_SCREENSHOT_PATH ??
    "/tmp/root-worker-electron-cdp-browser-panel.png";

  const requireFromApp = createRequire(path.join(appDir, "package.json"));
  const { chromium } = loadPlaywright(requireFromApp);
  const targetServer = await startTargetServer();

  const appProcess = spawn("pnpm", ["--dir", appDir, "exec", "electron", "."], {
    cwd: appDir,
    detached: process.platform !== "win32",
    env: { ...process.env },
    stdio: ["ignore", "pipe", "pipe"],
  });
  const appLogs = collectProcessLogs(appProcess);

  let cdpBrowser = null;
  let caughtError = null;
  try {
    cdpBrowser = await connectOverCdp(
      chromium,
      cdpPort,
      appProcess,
      appLogs,
      signal,
    );
    const appPage = await waitForCdpPage(cdpBrowser, devUrl, signal);

    await withAbort(
      appPage.waitForLoadState("domcontentloaded", { timeout: 30_000 }),
      signal,
    );
    await withAbort(
      appPage.waitForFunction(
        () => Boolean(globalThis.window?.codexDesktop?.navigateBrowserView),
        { timeout: 30_000 },
      ),
      signal,
    );
    await withAbort(
      appPage.evaluate(async ({ targetUrl }) => {
        await globalThis.window.codexDesktop.showBrowserView({
          x: 0,
          y: 0,
          width: 900,
          height: 640,
        });
        await globalThis.window.codexDesktop.navigateBrowserView(targetUrl);
      }, { targetUrl: targetServer.url }),
      signal,
    );

    const browserPanelPage = await waitForCdpPage(
      cdpBrowser,
      targetServer.url,
      signal,
    );
    const consoleLines = [];
    const networkUrls = [];
    browserPanelPage.on("console", (message) => {
      consoleLines.push(`${message.type()}: ${message.text()}`);
    });
    browserPanelPage.on("request", (request) => {
      networkUrls.push(request.url());
    });

    await withAbort(
      browserPanelPage.waitForLoadState("domcontentloaded", {
        timeout: 30_000,
      }),
      signal,
    );
    const marker = await withAbort(
      browserPanelPage.locator("#cdp-target").innerText({
        timeout: 10_000,
      }),
      signal,
    );
    await withAbort(browserPanelPage.locator("#cdp-button").click(), signal);
    const clicked = await withAbort(
      browserPanelPage.locator("#cdp-state").innerText({
        timeout: 10_000,
      }),
      signal,
    );
    await withAbort(
      browserPanelPage.screenshot({ path: screenshotPath, fullPage: true }),
      signal,
    );

    console.log(
      JSON.stringify(
        {
          cdpUrl: `http://127.0.0.1:${cdpPort}`,
          targetUrl: targetServer.url,
          targetPageUrl: browserPanelPage.url(),
          marker,
          clicked,
          pageCount: cdpBrowser
            .contexts()
            .flatMap((context) => context.pages())
            .length,
          screenshot: screenshotPath,
          consoleLines: consoleLines.slice(-8),
          networkUrls: networkUrls.slice(-8),
          appLogs: appLogs.slice(-12),
        },
        null,
        2,
      ),
    );
  } catch (error) {
    caughtError = withElectronLogs(error, appLogs);
  } finally {
    await withTimeout(cdpBrowser?.close(), 5_000).catch(() => {});
    await terminateProcess(appProcess, appLogs, "Electron");
    await new Promise((resolve) => targetServer.server.close(resolve));
  }
  if (caughtError) {
    throw caughtError;
  }
}

function loadPlaywright(requireFromApp) {
  try {
    return requireFromApp("playwright");
  } catch {
    // Keep the error below focused on the expected project dependency.
  }

  throw new Error(
    "Cannot find playwright from apps/root-worker-prototype. Run `pnpm install` in this checkout " +
      "so apps/root-worker-prototype/node_modules is available locally.",
  );
}

async function connectOverCdp(chromium, cdpPort, appProcess, appLogs, signal) {
  const cdpUrl = `http://127.0.0.1:${cdpPort}`;
  const deadline = Date.now() + 30_000;
  let lastError = null;
  while (Date.now() < deadline) {
    throwIfAborted(signal);
    if (appProcess.exitCode != null) {
      throw new Error(
        `Electron exited before CDP was available; code=${appProcess.exitCode}; logs=${JSON.stringify(appLogs)}`,
      );
    }
    try {
      return await chromium.connectOverCDP(cdpUrl, { timeout: 5_000 });
    } catch (error) {
      lastError = error;
      await delay(250, signal);
    }
  }
  const suffix = `logs=${JSON.stringify(appLogs.slice(-20))}`;
  if (lastError) {
    throw new Error(`${lastError.message}; ${suffix}`, { cause: lastError });
  }
  throw new Error(`Timed out connecting to ${cdpUrl}; ${suffix}`);
}

function collectProcessLogs(childProcess) {
  const lines = [];
  const append = (prefix, chunk) => {
    const text = chunk.toString("utf8");
    for (const line of text.split(/\r?\n/)) {
      if (line) {
        lines.push(`${prefix}: ${line}`);
      }
    }
    while (lines.length > 80) {
      lines.shift();
    }
  };
  childProcess.stdout.on("data", (chunk) => append("stdout", chunk));
  childProcess.stderr.on("data", (chunk) => append("stderr", chunk));
  return lines;
}

async function terminateProcess(childProcess, logs, label) {
  if (childProcess.exitCode != null || childProcess.signalCode != null) {
    return;
  }
  if (!signalProcess(childProcess, "SIGTERM")) {
    console.error(`${label} process was not running during cleanup`);
    return;
  }
  if (await waitForProcessExit(childProcess, 5_000)) {
    return;
  }
  console.error(
    `${label} did not exit after SIGTERM; sending SIGKILL; logs=${JSON.stringify(logs.slice(-20))}`,
  );
  signalProcess(childProcess, "SIGKILL");
  if (!(await waitForProcessExit(childProcess, 5_000))) {
    console.error(
      `${label} did not exit after SIGKILL; logs=${JSON.stringify(logs.slice(-20))}`,
    );
  }
}

function waitForProcessExit(childProcess, timeoutMs) {
  if (childProcess.exitCode != null || childProcess.signalCode != null) {
    return Promise.resolve(true);
  }
  return new Promise((resolve) => {
    const timeout = setTimeout(done, timeoutMs, false);
    childProcess.once("exit", onExit);

    function onExit() {
      done(true);
    }

    function done(result) {
      clearTimeout(timeout);
      childProcess.removeListener("exit", onExit);
      resolve(result);
    }
  });
}

function signalProcess(childProcess, signal) {
  try {
    if (process.platform !== "win32" && childProcess.pid) {
      process.kill(-childProcess.pid, signal);
      return true;
    }
    return childProcess.kill(signal);
  } catch {
    return false;
  }
}

async function waitForCdpPage(browser, targetUrl, signal) {
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    throwIfAborted(signal);
    for (const context of browser.contexts()) {
      for (const page of context.pages()) {
        if (page.url().startsWith(targetUrl)) {
          return page;
        }
      }
    }
    await delay(250, signal);
  }
  const urls = browser
    .contexts()
    .flatMap((context) => context.pages())
    .map((page) => page.url());
  throw new Error(
    `Timed out waiting for Browser panel CDP target ${targetUrl}; pages=${JSON.stringify(urls)}`,
  );
}

async function startTargetServer() {
  const server = createServer((request, response) => {
    response.writeHead(200, { "content-type": "text/html; charset=utf-8" });
    response.end(`<!doctype html>
<html>
  <head><title>CDP Browser Panel Target</title></head>
  <body>
    <main id="cdp-target">Root Worker Browser panel CDP target</main>
    <button id="cdp-button">Click</button>
    <output id="cdp-state">idle</output>
    <script>
      console.log("cdp target loaded");
      document.getElementById("cdp-button").addEventListener("click", () => {
        document.getElementById("cdp-state").textContent = "clicked";
        fetch("/ping").catch(() => {});
      });
    </script>
  </body>
</html>`);
  });
  await new Promise((resolve, reject) => {
    server.on("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const { port } = server.address();
  return {
    server,
    url: `http://127.0.0.1:${port}/`,
  };
}

function requiredEnv(name) {
  const value = process.env[name];
  if (!value) {
    throw new Error(`Missing required environment variable: ${name}`);
  }
  return value;
}

function delay(ms, signal) {
  throwIfAborted(signal);
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(done, ms);
    signal?.addEventListener("abort", onAbort, { once: true });

    function onAbort() {
      cleanup();
      reject(signal.reason ?? new Error("Smoke aborted"));
    }

    function done() {
      cleanup();
      resolve();
    }

    function cleanup() {
      clearTimeout(timeout);
      signal?.removeEventListener("abort", onAbort);
    }
  });
}

function parsePositiveIntegerEnv(name, fallback) {
  const value = process.env[name];
  if (!value) {
    return fallback;
  }
  const trimmed = value.trim();
  if (!/^\d+$/.test(trimmed)) {
    return fallback;
  }
  const parsed = Number(trimmed);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

function throwIfAborted(signal) {
  if (signal?.aborted) {
    throw signal.reason ?? new Error("Smoke aborted");
  }
}

function withTimeout(promise, timeoutMs) {
  if (!promise) {
    return Promise.resolve();
  }
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error("Timed out")), timeoutMs);
    promise.then(
      (value) => {
        clearTimeout(timeout);
        resolve(value);
      },
      (error) => {
        clearTimeout(timeout);
        reject(error);
      },
    );
  });
}

function withAbort(promise, signal) {
  throwIfAborted(signal);
  if (!signal) {
    return promise;
  }
  return new Promise((resolve, reject) => {
    signal.addEventListener("abort", onAbort, { once: true });
    promise.then(
      (value) => {
        cleanup();
        resolve(value);
      },
      (error) => {
        cleanup();
        reject(error);
      },
    );

    function onAbort() {
      cleanup();
      reject(signal.reason ?? new Error("Smoke aborted"));
    }

    function cleanup() {
      signal.removeEventListener("abort", onAbort);
    }
  });
}

function withElectronLogs(error, logs) {
  if (!(error instanceof Error)) {
    return error;
  }
  if (!error.message.includes("Electron logs=")) {
    error.message = `${error.message}; Electron logs=${JSON.stringify(logs.slice(-20))}`;
  }
  return error;
}

main().catch((error) => {
  console.error(error);
  process.exit(error.exitCode ?? 1);
});

#!/usr/bin/env node
"use strict";

const { createRequire } = require("node:module");
const path = require("node:path");

async function main() {
  const appDir = requiredEnv("APP_DIR");
  const devUrl = requiredEnv("ROOT_WORKER_DEV_SERVER_URL");
  const screenshotPath =
    process.env.ROOT_WORKER_SCREENSHOT_PATH ??
    "/tmp/root-worker-electron-playwright-app.png";
  const smokeText =
    process.env.ROOT_WORKER_SMOKE_INPUT ??
    "Playwright Electron debug smoke input";

  const requireFromApp = createRequire(path.join(appDir, "package.json"));
  const { _electron: electron } = loadPlaywright(requireFromApp);
  const executablePath = requireFromApp("electron");

  const app = await electron.launch({
    executablePath,
    cwd: appDir,
    args: ["."],
    env: { ...process.env },
    timeout: 45_000,
  });

  try {
    let page = await app
      .waitForEvent("window", {
        predicate: (candidate) => candidate.url().startsWith(devUrl),
        timeout: 45_000,
      })
      .catch(async () => {
        const match = app
          .windows()
          .find((candidate) => candidate.url().startsWith(devUrl));
        return match ?? app.firstWindow({ timeout: 45_000 });
      });

    const consoleLines = [];
    page.on("console", (message) => {
      consoleLines.push(`${message.type()}: ${message.text()}`);
    });

    await page.waitForLoadState("domcontentloaded", { timeout: 30_000 });
    await page.waitForTimeout(2_500);

    if (!page.url().startsWith(devUrl)) {
      const match = app
        .windows()
        .find((candidate) => candidate.url().startsWith(devUrl));
      if (match) {
        page = match;
      }
    }

    const hasDesktop = await page.evaluate(() =>
      Boolean(globalThis.window?.codexDesktop),
    );
    const title = await page.title();
    const url = page.url();
    const bodyText = await page
      .locator("body")
      .innerText({ timeout: 5_000 })
      .catch((error) => `BODY_READ_FAILED:${error.message}`);
    const controls = await page
      .locator("textarea, input, [contenteditable=true]")
      .count();

    let typed = false;
    for (let index = 0; index < controls; index += 1) {
      const control = page
        .locator("textarea, input, [contenteditable=true]")
        .nth(index);
      if (await control.isVisible().catch(() => false)) {
        await control.click();
        await page.keyboard.type(smokeText);
        typed = true;
        break;
      }
    }

    await page.screenshot({ path: screenshotPath, fullPage: true });

    const windowUrls = app.windows().map((candidate) => candidate.url());
    const windowCount = await app.evaluate(
      ({ BrowserWindow }) => BrowserWindow.getAllWindows().length,
    );

    console.log(
      JSON.stringify(
        {
          title,
          url,
          hasDesktop,
          controls,
          typed,
          windowCount,
          windowUrls,
          screenshot: screenshotPath,
          bodyPreview: bodyText.slice(0, 200),
          consoleLines: consoleLines.slice(-8),
        },
        null,
        2,
      ),
    );

    if (!hasDesktop) {
      throw new Error("window.codexDesktop is not available");
    }
  } finally {
    await app.close();
  }
}

function loadPlaywright(requireFromApp) {
  try {
    return requireFromApp("playwright");
  } catch {
    // Keep the error below focused on the expected project dependency.
  }

  throw new Error(
    "Cannot find playwright from apps/root-worker-prototype. Run `pnpm install` in the main checkout, " +
      "then bootstrap linked worktrees so apps/root-worker-prototype/node_modules is reused.",
  );
}

function requiredEnv(name) {
  const value = process.env[name];
  if (!value) {
    throw new Error(`Missing required environment variable: ${name}`);
  }
  return value;
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});

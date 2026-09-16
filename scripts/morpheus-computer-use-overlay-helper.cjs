#!/usr/bin/env electron
"use strict";

const readline = require("node:readline");
const { app, BrowserWindow, screen } = require("electron");
const {
  createComputerUseOverlayController,
} = require("../apps/root-worker-prototype/electron/computerUseOverlay.cjs");

let controller = null;

configureNonActivatingHelperApp();

function send(message) {
  process.stdout.write(`${JSON.stringify(message)}\n`);
}

async function controllerForApp() {
  await app.whenReady();
  configureNonActivatingHelperApp();
  if (!controller) {
    controller = createComputerUseOverlayController({
      BrowserWindow,
      hostApp: app,
      screen,
      logger: console,
      restoreActivationPolicy: false,
    });
  }
  return controller;
}

async function handleMessage(message) {
  if (!message || typeof message !== "object") {
    throw new Error("Overlay helper message must be an object");
  }
  if (message.type === "update") {
    const overlay = await controllerForApp();
    await overlay.update(message.payload ?? {});
    return { available: true, visible: true };
  }
  if (message.type === "destroy") {
    await controller?.destroy?.();
    controller = null;
    return { available: true, visible: false, destroyed: true };
  }
  if (message.type === "quit") {
    await controller?.destroy?.();
    controller = null;
    return { available: true, visible: false, quitting: true };
  }
  throw new Error(`Unsupported overlay helper message: ${String(message.type)}`);
}

const lines = readline.createInterface({
  input: process.stdin,
  crlfDelay: Number.POSITIVE_INFINITY,
});

lines.on("line", (line) => {
  let message;
  try {
    message = JSON.parse(line);
  } catch (error) {
    send({ ok: false, error: errorMessage(error) });
    return;
  }
  void handleMessage(message)
    .then((result) => {
      send({ id: message.id ?? null, ok: true, ...result });
      if (message.type === "quit") {
        setImmediate(() => app.quit());
      }
    })
    .catch((error) =>
      send({ id: message?.id ?? null, ok: false, error: errorMessage(error) }),
    );
});

lines.on("close", () => {
  void controller?.destroy?.().finally(() => app.quit());
});

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}

function configureNonActivatingHelperApp() {
  try {
    app.setActivationPolicy?.("accessory");
  } catch {}
  try {
    const result = app.dock?.hide?.();
    if (result && typeof result.catch === "function") {
      result.catch(() => {});
    }
  } catch {}
}

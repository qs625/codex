"use strict";

const OVERLAY_HTML = `<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <style>
    html, body {
      width: 100%;
      height: 100%;
      margin: 0;
      overflow: hidden;
      background: transparent;
      pointer-events: none;
    }
    #trail {
      position: absolute;
      inset: 0;
      width: 100%;
      height: 100%;
      overflow: visible;
    }
    #path {
      fill: none;
      stroke: rgba(45, 212, 191, 0.48);
      stroke-width: 3;
      stroke-linecap: round;
      stroke-linejoin: round;
      filter: drop-shadow(0 0 5px rgba(45, 212, 191, 0.38));
    }
    #cursor {
      position: absolute;
      left: 0;
      top: 0;
      width: 18px;
      height: 18px;
      transform: translate(-5px, -5px);
      transition-property: left, top;
      transition-timing-function: cubic-bezier(0.22, 1, 0.36, 1);
    }
    #cursor::before {
      content: "";
      position: absolute;
      left: 0;
      top: 0;
      width: 0;
      height: 0;
      border-left: 14px solid #f8fafc;
      border-top: 3px solid transparent;
      border-bottom: 12px solid transparent;
      filter: drop-shadow(0 1px 2px rgba(15, 23, 42, 0.8));
    }
    #cursor::after {
      content: "";
      position: absolute;
      left: 1px;
      top: 1px;
      width: 0;
      height: 0;
      border-left: 8px solid #14b8a6;
      border-top: 2px solid transparent;
      border-bottom: 7px solid transparent;
    }
  </style>
</head>
<body>
  <svg id="trail"><polyline id="path" points=""></polyline></svg>
  <div id="cursor"></div>
  <script>
    const cursor = document.getElementById("cursor");
    const path = document.getElementById("path");
    let trailTimer = null;
    window.__setComputerUseCursor = (payload) => {
      const point = payload && payload.agentCursor;
      if (!point) {
        cursor.style.display = "none";
        path.setAttribute("points", "");
        return;
      }
      cursor.style.display = "block";
      cursor.style.transitionDuration = String(payload.durationMs || 0) + "ms";
      cursor.style.left = String(point.x - payload.bounds.x) + "px";
      cursor.style.top = String(point.y - payload.bounds.y) + "px";
      const samples = Array.isArray(payload.pathSamples) ? payload.pathSamples : [];
      path.setAttribute(
        "points",
        samples.map((sample) => String(sample.x - payload.bounds.x) + "," + String(sample.y - payload.bounds.y)).join(" ")
      );
      if (trailTimer) {
        clearTimeout(trailTimer);
      }
      trailTimer = setTimeout(() => path.setAttribute("points", ""), 900);
    };
  </script>
</body>
</html>`;

function createComputerUseOverlayController(options = {}) {
  return new ComputerUseOverlayController(options);
}

class ComputerUseOverlayController {
  constructor({ BrowserWindow, hostApp, screen, logger = console } = {}) {
    this.BrowserWindow = BrowserWindow;
    this.hostApp = hostApp ?? null;
    this.screen = screen;
    this.logger = logger;
    this.window = null;
    this.readyPromise = null;
  }

  async update(payload) {
    if (!payload?.agentCursor) {
      await this.destroy();
      return;
    }
    const overlayWindow = this.ensureWindow();
    const bounds = displayUnionBounds(this.screen);
    overlayWindow.setBounds(bounds);
    if (typeof overlayWindow.showInactive === "function") {
      overlayWindow.showInactive();
    } else {
      overlayWindow.show();
    }
    restoreRegularActivationPolicy(this.hostApp);
    await this.readyPromise;
    const overlayPayload = {
      agentCursor: payload.agentCursor,
      bounds,
      durationMs: payload.durationMs ?? 0,
      pathSamples: payload.pathSamples ?? [],
      status: payload.status ?? "active",
    };
    await overlayWindow.webContents.executeJavaScript(
      `window.__setComputerUseCursor(${JSON.stringify(overlayPayload)})`,
    );
  }

  async destroy() {
    const overlayWindow = this.window;
    this.window = null;
    this.readyPromise = null;
    if (overlayWindow && !overlayWindow.isDestroyed()) {
      overlayWindow.close();
    }
    restoreRegularActivationPolicy(this.hostApp);
  }

  ensureWindow() {
    if (this.window && !this.window.isDestroyed()) {
      return this.window;
    }
    if (!this.BrowserWindow || !this.screen) {
      throw new Error("Computer Use overlay requires Electron BrowserWindow and screen");
    }
    const overlayWindow = new this.BrowserWindow({
      ...displayUnionBounds(this.screen),
      frame: false,
      transparent: true,
      resizable: false,
      movable: false,
      focusable: false,
      skipTaskbar: true,
      hasShadow: false,
      alwaysOnTop: true,
      fullscreenable: false,
      webPreferences: {
        contextIsolation: true,
        nodeIntegration: false,
      },
    });
    overlayWindow.setIgnoreMouseEvents(true, { forward: true });
    overlayWindow.setAlwaysOnTop(true, "screen-saver");
    if (typeof overlayWindow.setVisibleOnAllWorkspaces === "function") {
      overlayWindow.setVisibleOnAllWorkspaces(true, {
        visibleOnFullScreen: true,
      });
    }
    restoreRegularActivationPolicy(this.hostApp);
    this.readyPromise = overlayWindow
      .loadURL(`data:text/html;charset=utf-8,${encodeURIComponent(OVERLAY_HTML)}`)
      .catch((error) => {
        this.logger.warn?.("Computer Use overlay failed to load", error);
        throw error;
      });
    this.window = overlayWindow;
    return overlayWindow;
  }
}

function restoreRegularActivationPolicy(hostApp) {
  if (!hostApp) {
    return;
  }
  try {
    hostApp.setActivationPolicy?.("regular");
  } catch {}
  try {
    const result = hostApp.dock?.show?.();
    if (result && typeof result.catch === "function") {
      result.catch(() => {});
    }
  } catch {}
}

function displayUnionBounds(screen) {
  const displays =
    typeof screen?.getAllDisplays === "function" ? screen.getAllDisplays() : [];
  const boundsList = displays
    .map((display) => display.bounds)
    .filter((bounds) => bounds && Number.isFinite(bounds.width) && Number.isFinite(bounds.height));
  if (boundsList.length === 0) {
    return { x: 0, y: 0, width: 1, height: 1 };
  }
  const minX = Math.min(...boundsList.map((bounds) => bounds.x));
  const minY = Math.min(...boundsList.map((bounds) => bounds.y));
  const maxX = Math.max(...boundsList.map((bounds) => bounds.x + bounds.width));
  const maxY = Math.max(...boundsList.map((bounds) => bounds.y + bounds.height));
  return {
    x: minX,
    y: minY,
    width: maxX - minX,
    height: maxY - minY,
  };
}

module.exports = {
  OVERLAY_HTML,
  createComputerUseOverlayController,
  displayUnionBounds,
  restoreRegularActivationPolicy,
};

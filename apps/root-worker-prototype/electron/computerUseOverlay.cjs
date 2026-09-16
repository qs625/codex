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
      stroke: url(#agent-trail-gradient);
      stroke-width: 2;
      stroke-linecap: round;
      stroke-linejoin: round;
      opacity: 0.72;
      filter: drop-shadow(0 0 4px rgba(34, 211, 238, 0.28));
    }
    #trail-tip {
      fill: #ecfeff;
      stroke: rgba(20, 184, 166, 0.8);
      stroke-width: 1.4;
      opacity: 0;
      filter: drop-shadow(0 0 5px rgba(45, 212, 191, 0.35));
    }
    #cursor {
      position: absolute;
      left: 0;
      top: 0;
      width: 24px;
      height: 24px;
      transform: translate(-12px, -12px);
      transition-property: left, top, opacity;
      transition-timing-function: cubic-bezier(0.22, 1, 0.36, 1);
    }
    .cursor-halo {
      content: "";
      position: absolute;
      inset: 1px;
      border-radius: 999px;
      background: radial-gradient(circle, rgba(236, 254, 255, 0.9) 0%, rgba(34, 211, 238, 0.24) 45%, rgba(20, 184, 166, 0) 72%);
      filter: blur(1.5px);
      opacity: 0.74;
    }
    .cursor-gem {
      content: "";
      position: absolute;
      left: 6px;
      top: 3px;
      width: 12px;
      height: 17px;
      border: 1px solid rgba(8, 145, 178, 0.72);
      border-radius: 7px 7px 8px 7px;
      background: linear-gradient(145deg, #ffffff 0%, #ecfeff 38%, #2dd4bf 100%);
      box-shadow:
        0 1px 2px rgba(15, 23, 42, 0.38),
        0 0 0 1px rgba(255, 255, 255, 0.74) inset,
        0 0 10px rgba(45, 212, 191, 0.24);
      transform: rotate(45deg) skew(-5deg, -5deg);
      transform-origin: 50% 64%;
    }
    .cursor-gem::after {
      content: "";
      position: absolute;
      left: 3px;
      top: 2px;
      width: 5px;
      height: 8px;
      border-radius: 999px;
      background: rgba(255, 255, 255, 0.72);
      transform: rotate(-10deg);
    }
    .cursor-dot {
      content: "";
      position: absolute;
      left: 10px;
      top: 9px;
      width: 4px;
      height: 4px;
      border-radius: 999px;
      background: #0f766e;
      box-shadow: 0 0 0 2px rgba(236, 254, 255, 0.86);
    }
  </style>
</head>
<body>
  <svg id="trail">
    <defs>
      <linearGradient id="agent-trail-gradient" x1="0%" y1="0%" x2="100%" y2="0%">
        <stop offset="0%" stop-color="#22d3ee" stop-opacity="0" />
        <stop offset="48%" stop-color="#22d3ee" stop-opacity="0.38" />
        <stop offset="100%" stop-color="#2dd4bf" stop-opacity="0.78" />
      </linearGradient>
    </defs>
    <polyline id="path" points=""></polyline>
    <circle id="trail-tip" r="3.1" cx="0" cy="0"></circle>
  </svg>
  <div id="cursor" aria-hidden="true">
    <span class="cursor-halo"></span>
    <span class="cursor-gem"></span>
    <span class="cursor-dot"></span>
  </div>
  <script>
    const cursor = document.getElementById("cursor");
    const path = document.getElementById("path");
    const trailTip = document.getElementById("trail-tip");
    let trailTimer = null;
    window.__setComputerUseCursor = (payload) => {
      const point = payload && payload.agentCursor;
      if (!point) {
        cursor.style.display = "none";
        path.setAttribute("points", "");
        trailTip.style.opacity = "0";
        return;
      }
      cursor.style.display = "block";
      cursor.style.transitionDuration = String(payload.durationMs || 0) + "ms";
      const localX = point.x - payload.bounds.x;
      const localY = point.y - payload.bounds.y;
      cursor.style.left = String(localX) + "px";
      cursor.style.top = String(localY) + "px";
      const samples = Array.isArray(payload.pathSamples) ? payload.pathSamples : [];
      path.setAttribute(
        "points",
        samples.map((sample) => String(sample.x - payload.bounds.x) + "," + String(sample.y - payload.bounds.y)).join(" ")
      );
      trailTip.setAttribute("cx", String(localX));
      trailTip.setAttribute("cy", String(localY));
      trailTip.style.opacity = samples.length > 1 ? "0.84" : "0";
      if (trailTimer) {
        clearTimeout(trailTimer);
      }
      trailTimer = setTimeout(() => {
        path.setAttribute("points", "");
        trailTip.style.opacity = "0";
      }, 900);
    };
  </script>
</body>
</html>`;

function createComputerUseOverlayController(options = {}) {
  return new ComputerUseOverlayController(options);
}

class ComputerUseOverlayController {
  constructor({
    BrowserWindow,
    hostApp,
    screen,
    logger = console,
    restoreActivationPolicy = true,
  } = {}) {
    this.BrowserWindow = BrowserWindow;
    this.hostApp = hostApp ?? null;
    this.screen = screen;
    this.logger = logger;
    this.restoreActivationPolicy = restoreActivationPolicy;
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
    this.restoreRegularActivationPolicy();
    await this.readyPromise;
    const overlayPayload = {
      agentCursor: payload.agentCursor,
      bounds,
      durationMs: payload.durationMs ?? 0,
      pathSamples: payload.pathSamples ?? [],
      status: payload.status ?? "active",
      targetBounds: payload.targetBounds ?? null,
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
    this.restoreRegularActivationPolicy();
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
      show: false,
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
    this.restoreRegularActivationPolicy();
    this.readyPromise = overlayWindow
      .loadURL(`data:text/html;charset=utf-8,${encodeURIComponent(OVERLAY_HTML)}`)
      .catch((error) => {
        this.logger.warn?.("Computer Use overlay failed to load", error);
        throw error;
      });
    this.window = overlayWindow;
    return overlayWindow;
  }

  restoreRegularActivationPolicy() {
    if (this.restoreActivationPolicy) {
      restoreRegularActivationPolicy(this.hostApp);
    }
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

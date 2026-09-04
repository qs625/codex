---
name: client-browser-cdp-debug
description: "Use when debugging another frontend page inside the Root Worker client browser with Playwright/CDP: open the page in the built-in Browser panel, attach to the Electron/Chromium CDP endpoint, inspect DOM/console/network, capture screenshots, and keep the client browser as the debugging surface instead of launching an external browser."
---

# Client Browser CDP Debug

Use this skill when the user wants Playwright/CDP debugging for a frontend page opened in the Root Worker client browser.

## Boundaries

- Use the Root Worker client's built-in Browser panel as the page host.
- Do not make `playwright open --browser chromium <url>` the main path; that debugs an external browser and misses client-browser behavior.
- Keep Browser panel URL guard and `WebContentsView` security settings unchanged.
- CDP is explicit opt-in only. Never enable it for normal or packaged usage by default.
- CDP must bind to `127.0.0.1`.

## Reuse Existing Skills

- For generic Playwright/CDP operations, use `bytedance-frontend-debug`.
- For complete Root Worker Electron launch rules, use `root-worker-playwright-debug`.
- This skill only bridges those flows: enable the client Electron CDP endpoint, open the target URL in the Browser panel, then select that Browser panel page target from CDP.

## Fast Path

From this skill directory, run:

```bash
rtk scripts/run-client-browser-cdp-smoke.sh
```

The script starts the full Root Worker Electron client, enables `ROOT_WORKER_REMOTE_DEBUGGING_PORT` on a free loopback port, opens a local http target in the Browser panel, connects with `chromium.connectOverCDP()`, and prints JSON with `cdpUrl`, `targetPageUrl`, console lines, network URLs, and screenshot path.

## Manual Flow

1. Start the full Root Worker Electron client with CDP enabled:

```bash
rtk env ROOT_WORKER_ENABLE_CDP=1 .codex/skills/root-worker-playwright-debug/scripts/launch-electron-dev.sh
```

2. Read the printed `CDP_URL=http://127.0.0.1:<port>`.
3. In the Root Worker Browser panel, open the frontend URL to debug.
4. Use Playwright `chromium.connectOverCDP(CDP_URL)`.
5. Select the page whose URL matches the Browser panel target.
6. Use normal Playwright actions for DOM inspection, console/network capture, screenshots, and interaction.

For reusable validation, prefer this skill's script over hand-written one-off CDP attach code.

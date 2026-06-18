---
name: root-worker-playwright-debug
description: "在 my-codex 项目中使用 Playwright 调试 root-worker prototype 的完整 Electron 客户端。适用于 Electron window、renderer DOM/console/network、preload IPC、app-server stdio、随机 Vite dev port、专用共享 CODEX_HOME、CODEX_APP_SERVER_CMD 与 ROOT_WORKER_WORKSPACE 配置。"
---

# Root Worker Playwright 调试

## 核心规则

不要用 Playwright 直接打开 Vite server 页面调试 root-worker。浏览器打开 `http://127.0.0.1:<port>` 只覆盖 renderer，缺少 Electron preload 注入的 `window.codexDesktop`、IPC、app-server stdio、文件系统和主进程生命周期。

调试时一律启动完整 Electron 应用，再用 Playwright 控制 Electron window。Vite dev server 只是 Electron window 加载 renderer 的内部依赖，不是 Playwright 的调试目标。

## 依赖准备

`playwright` 是 `apps/root-worker-prototype` 的 devDependency。主 checkout 首次使用前准备 JS 依赖：

```bash
rtk pnpm install
```

固定开发 checkout `~/Projects/my-codex-dev` 是否复用 `apps/root-worker-prototype/node_modules` 取决于当前目录状态；如果没有可用依赖，先在对应 checkout 确认依赖目录或运行必要的 JS 依赖安装命令。

脚本会从 `apps/root-worker-prototype/package.json` 所在目录解析 `playwright` 和 `electron`，不要依赖全局 Playwright，也不要从临时 runtime 目录运行脚本。

## 固定脚本

本 skill 的可执行脚本都在：

```text
scripts/
```

以下脚本路径都相对本 skill 目录。使用脚本时先解析 skill 目录，再运行对应 `scripts/...` 文件。

### Smoke Test

运行一次完整 Electron smoke：随机端口启动 Vite，启动 Electron，关闭 DevTools，选择真实应用窗口，检查 `window.codexDesktop`，尝试输入，并截图。

```bash
rtk scripts/run-electron-smoke.sh
```

默认输出：

- 截图：`/tmp/root-worker-electron-playwright-app.png`
- Vite 日志：`/tmp/root-worker-vite-playwright.log`
- JSON 摘要写到 stdout，包括 `title`、`url`、`hasDesktop`、`typed`、`windowUrls`

常用覆盖：

```bash
rtk env \
  ROOT_WORKER_SCREENSHOT_PATH=/tmp/root-worker-debug.png \
  ROOT_WORKER_SMOKE_INPUT="hello from playwright" \
  scripts/run-electron-smoke.sh
```

### 手动调试

启动一个可手动操作的完整 Electron dev 实例。脚本会自动选空闲端口，并把 Vite URL 传给 Electron。

```bash
rtk scripts/launch-electron-dev.sh
```

默认关闭 DevTools，避免 Playwright 抓到 DevTools window。需要 DevTools 时：

```bash
rtk env ROOT_WORKER_OPEN_DEVTOOLS=1 \
  scripts/launch-electron-dev.sh
```

## Codex 状态

调试实例不要混用当前正在运行客户端的 `CODEX_HOME`。默认使用专用共享目录：

```text
/tmp/my-codex-root-worker-debug/codex-home
```

多个 worktree 的调试实例可以共享这个目录，方便复现同一批线程和配置；它和当前正在运行的 Codex 客户端状态隔离。

相关默认路径：

```text
CODEX_HOME=/tmp/my-codex-root-worker-debug/codex-home
ROOT_WORKER_WORKSPACE=/tmp/my-codex-root-worker-debug/workspace
```

如需冷启动或隔离某次调试：

```bash
rtk env \
  ROOT_WORKER_DEBUG_CODEX_HOME=/tmp/root-worker-oneoff/codex-home \
  ROOT_WORKER_DEBUG_WORKSPACE=/tmp/root-worker-oneoff/workspace \
  scripts/run-electron-smoke.sh
```

## App Server

Electron app-server command 选择逻辑：

1. 优先使用 `CODEX_APP_SERVER_CMD`
2. 否则从 Electron 文件目录向上查找 `codex-rs/target/debug/codex-app-server`
3. 找不到 workspace binary 时 fallback 到 PATH 上的 `codex-app-server`

固定脚本默认显式使用当前 repo 的 debug binary：

```text
CODEX_APP_SERVER_CMD="$REPO/codex-rs/target/debug/codex-app-server --listen stdio://"
```

如需指定其他 binary：

```bash
rtk env CODEX_APP_SERVER_CMD="/path/to/codex-app-server --listen stdio://" \
  scripts/run-electron-smoke.sh
```

## 端口策略

默认 `pnpm --filter @my-codex/root-worker-prototype dev` 使用固定 `5173` 且 `strictPort: true`。多个 dev 实例会抢端口。

固定脚本会：

- 自动选择一个空闲 `127.0.0.1` 端口
- 用这个端口启动 Vite
- 设置 `ROOT_WORKER_DEV_SERVER_URL=http://127.0.0.1:<port>`
- 启动 Electron dev mode 加载同一个 URL

不要假设 Vite 会自动换端口。

## Playwright 关键点

- 使用 `_electron.launch()` 启动完整应用。
- 设置 `ROOT_WORKER_OPEN_DEVTOOLS=0`，避免抓到 DevTools window。
- 通过 URL 匹配 `ROOT_WORKER_DEV_SERVER_URL` 获取真实应用窗口。
- 用 `page.evaluate(() => Boolean(window.codexDesktop))` 验证 preload IPC。
- 用 `page.locator(...)`、`page.getByText(...)` 检查真实 UI。
- 用 `page.on("console", ...)` 收集 renderer console。
- 用 `app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().length)` 检查主进程窗口状态。

## 脚本文件

- `scripts/run-electron-smoke.sh`：完整 smoke 入口。
- `scripts/electron-smoke.cjs`：Playwright Electron 自动化逻辑。
- `scripts/launch-electron-dev.sh`：手动完整 Electron dev 启动入口。

## 关键项目文件

- `apps/root-worker-prototype/package.json`：默认 `dev` 启动 Vite + Electron。
- `apps/root-worker-prototype/vite.config.ts`：固定 `127.0.0.1:5173` 且 `strictPort: true`。
- `apps/root-worker-prototype/electron/main.cjs`：Electron 主进程入口；支持 `ROOT_WORKER_DEV_SERVER_URL` 和 `ROOT_WORKER_OPEN_DEVTOOLS`。
- `apps/root-worker-prototype/electron/preload.cjs`：暴露 `window.codexDesktop`。
- `apps/root-worker-prototype/electron/appServerClient.cjs`：app-server command、`CODEX_HOME` 和 stdio client。

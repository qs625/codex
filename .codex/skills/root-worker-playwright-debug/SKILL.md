---
name: root-worker-playwright-debug
description: "在 my-codex 项目中使用 Playwright/Playwright CLI 测试、调试 root-worker prototype。适用于调试 apps/root-worker-prototype 的 Vite renderer、浏览器 DOM/console/network、Electron 完整客户端边界、5173 端口冲突、CODEX_HOME 状态复用或隔离、CODEX_APP_SERVER_CMD 与 ROOT_WORKER_WORKSPACE 配置。"
---

# Root Worker Playwright 调试

## 适用场景

- 需要用 Playwright CLI 或浏览器自动化查看 `apps/root-worker-prototype` 的 renderer 布局、DOM、console、network。
- 需要判断某个问题是否必须跑完整 Electron 客户端才能复现。
- 需要启动 root-worker prototype 时处理 Vite `5173` 端口冲突。
- 需要决定调试实例复用当前 Codex 状态，还是用独立 `CODEX_HOME` 冷启动。
- 需要覆盖 app-server binary、工作目录或 Electron preload IPC 行为。

## 先判断调试目标

1. 如果目标是布局、CSS、React state、DOM、console 或 network，优先调 Vite renderer。
2. 如果目标依赖 `window.codexDesktop`、preload IPC、native dialog、app-server stdio、文件读取、LSP、realtime 或 Electron 主进程生命周期，必须调完整 Electron。
3. 如果目标是端到端自动化测试，先说明当前仓库没有现成 Playwright E2E 配置或 Playwright 依赖；不要假设可以直接运行 `playwright test`。

## Renderer-only：用 Playwright CLI 调 Vite 页面

普通浏览器打开 `http://127.0.0.1:5173` 只能覆盖 renderer/layout。它没有 Electron preload 注入的 `window.codexDesktop`，因此不能覆盖完整客户端功能。

流程：

1. 确认是否已经有 Vite server 在 `127.0.0.1:5173`。
   - 如果已经有，复用它，不要再启动第二个完整 dev 实例。
   - 如果没有，从仓库根目录启动：

   ```bash
   rtk pnpm --filter @my-codex/root-worker-prototype dev
   ```

2. 用 Playwright CLI 或浏览器自动化打开：

   ```bash
   http://127.0.0.1:5173
   ```

3. 验证点：
   - 页面能加载 renderer。
   - console 没有新的 React/Vite 错误。
   - network 请求符合预期。
   - 如果功能报错提示 `window.codexDesktop` 缺失，切换到完整 Electron 路径验证。

## 端口冲突处理

`apps/root-worker-prototype/vite.config.ts` 固定：

- host: `127.0.0.1`
- port: `5173`
- strictPort: `true`

因此同一台机器上多个完整 `dev` 实例会争抢 `5173`。处理顺序：

1. 如果已有 dev server 服务的是同一份代码，直接复用 `http://127.0.0.1:5173`。
2. 如果必须跑另一个 worktree 的完整 dev 实例，先停掉已有占用者。
3. 不要临时假设 Vite 会自动换端口；当前配置不会。
4. 换端口需要先改项目配置和 Electron dev URL 支持，这不属于普通调试流程。

## 完整 Electron：覆盖 preload IPC 和 app-server

完整客户端调试需要 Electron，因为 `apps/root-worker-prototype/electron/preload.cjs` 通过 `contextBridge` 暴露 `window.codexDesktop`，浏览器 renderer 没有这些 IPC。

启动完整客户端：

```bash
rtk pnpm --filter @my-codex/root-worker-prototype dev
```

这个脚本会并行启动：

- Vite renderer。
- Electron 主进程。
- Electron 内部 app-server client。

app-server command 选择逻辑：

1. 优先使用 `CODEX_APP_SERVER_CMD`。
2. 否则从 Electron 文件目录向上逐级查找 `codex-rs/target/debug/codex`。
3. 找不到 workspace binary 时，fallback 到 PATH 上的 `codex`。

只有在需要指定非默认 codex binary、调试包装脚本或确认 app-server 参数时，才设置 `CODEX_APP_SERVER_CMD`：

```bash
rtk env CODEX_APP_SERVER_CMD="codex app-server --listen stdio://" pnpm --filter @my-codex/root-worker-prototype dev
```

可选设置工作目录：

```bash
rtk env ROOT_WORKER_WORKSPACE=/path/to/workspace pnpm --filter @my-codex/root-worker-prototype dev
```

## Codex 状态：默认复用，按需隔离

root-worker prototype 默认 `CODEX_HOME` 是 `~/.codex-home`。在本项目调试中，多个 worktree 共用同一个 Codex 状态通常可以接受，不要强制隔离。

只有这些情况建议设置独立 `CODEX_HOME`：

- 需要冷启动、空历史或独立 auth/config。
- 需要避免调试过程污染常用会话状态。
- 需要复现首次启动或迁移行为。

示例：

```bash
rtk env CODEX_HOME=/tmp/my-codex-root-worker-ui-env/codex-home \
  ROOT_WORKER_WORKSPACE=/tmp/my-codex-root-worker-ui-env/workspace \
  pnpm --filter @my-codex/root-worker-prototype dev
```

## Playwright Electron API 方向

Playwright 本身支持通过 `_electron.launch()` 启动和调试 Electron 应用，可以用于完整客户端自动化。但当前仓库没有现成 Playwright E2E harness、`playwright.config.*` 或 Playwright 依赖，因此不要把它描述成开箱即用。

如果要新增 Electron E2E，需要先单独设计并引入：

- Playwright 依赖和配置。
- Electron launch 入口，通常指向 `apps/root-worker-prototype/electron/main.cjs` 或 package entry。
- app-server、`CODEX_HOME`、`ROOT_WORKER_WORKSPACE` 的测试隔离策略。
- 端口复用或动态端口能力。
- CI 运行环境和 Electron 可执行依赖。

## 常见判断

- 页面样式错位：先用 renderer-only。
- 点击后无响应且 console 提到 `codexDesktop`：改用完整 Electron。
- 启动时报 `Port 5173 is already in use`：复用现有 server 或停掉占用者。
- app-server 行为看起来不是本 worktree 代码：检查是否已构建 `codex-rs/target/debug/codex`，或显式设置 `CODEX_APP_SERVER_CMD`。
- 想验证全新账号/空会话：设置独立 `CODEX_HOME`。

## 关键项目文件

- `apps/root-worker-prototype/package.json`：`dev` 启动 Vite + Electron。
- `apps/root-worker-prototype/vite.config.ts`：固定 `127.0.0.1:5173` 且 `strictPort: true`。
- `apps/root-worker-prototype/electron/main.cjs`：Electron 主进程入口。
- `apps/root-worker-prototype/electron/preload.cjs`：暴露 `window.codexDesktop`。
- `apps/root-worker-prototype/electron/appServerClient.cjs`：app-server command、`CODEX_HOME` 和 stdio client。
- `apps/root-worker-prototype/README.md`：手动启动说明。

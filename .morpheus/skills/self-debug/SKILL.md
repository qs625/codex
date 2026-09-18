---
name: self-debug
description: "调试当前正在运行的 Morpheus/Root Worker 客户端自身时使用。必须配合 frontend-debug 和 playwright-cli：连接客户端暴露的 CDP endpoint，用 Playwright CLI 检查当前 Electron renderer、右侧 Browser panel、内置 web tab、DOM、console、network、截图和交互。"
---

# Self Debug

## 核心规则

本 skill 用于调试“当前正在运行的 Morpheus 客户端自己”，不是启动另一个 dev Electron，也不是打开外部 Chrome。

触发本 skill 时，必须同时使用：

- `frontend-debug`
- `playwright-cli`

先按 `frontend-debug` 的流程连接客户端暴露的 CDP endpoint：

```bash
playwright-cli attach --cdp=http://127.0.0.1:9222
```

如果客户端日志或启动输出给出了其它 CDP URL，使用实际 URL 替换 `127.0.0.1:9222`。

## 调试客户端自身

连接后先列 target：

```bash
playwright-cli --s=default tab-list
```

选择 `Root Worker Prototype` / Morpheus 主 renderer target 后，可用标准 Playwright CLI 检查 UI：

```bash
playwright-cli --s=default snapshot
playwright-cli --s=default find "Browser"
playwright-cli --s=default eval "document.title"
playwright-cli --s=default console
playwright-cli --s=default requests
```

需要确认是否连到内置 Electron，而不是外部浏览器时，检查：

```bash
lsof -nP -iTCP:9222 -sTCP:LISTEN
playwright-cli --s=default --raw eval "navigator.userAgent"
```

期望监听进程是 `Root Worker Runtime` / Electron app，UA 中包含 `Electron/...`。

## 调试内置 Browser panel 里的网页

优先使用 Morpheus 右侧 `Browser` panel 创建或打开网页 tab；不要让 Playwright 自己打开外部 Chrome。

当前兼容边界：

- 已存在的 Browser panel web tab 会出现在同一个 CDP target list 中；
- 可用 `tab-select`、`snapshot`、`eval`、`requests` 调试该 web target；
- 如果 `playwright-cli tab-new <url>` 暂时失败，先通过 Browser panel 地址栏打开 URL，再重新 `tab-list` 选择新 target。

典型流程：

```bash
playwright-cli --s=default tab-list
playwright-cli --s=default tab-select <web-target-index>
playwright-cli --s=default snapshot
playwright-cli --s=default --raw eval "JSON.stringify({ href: location.href, title: document.title })"
playwright-cli --s=default requests --static
```

## 约束

- 不要用 `playwright-cli open` 作为默认路径；它会启动/控制外部浏览器，不等价于调试当前客户端。
- 不要把 Vite 页面当成完整客户端；完整客户端能力依赖 Electron preload、IPC、app-server 和 Runtime Capsule。
- 不要绕过 Browser panel 的 URL 安全策略；调试网页时只使用客户端允许的 `http` / `https` / local dev URL。
- 如果需要启动隔离 dev Electron 实例，而不是调试当前运行客户端，先在当前任务中明确说明要调试 dev 实例，再按项目脚本或测试命令启动完整 Electron；不要把外部浏览器当成完整客户端。

# UI/UE Designer 截图工作流

## 任务 brief

`ui-ue-designer` agent 在处理现有 root-worker prototype 客户端 UI 设计、改造或评审时，需要基于当前真实界面，而不是只产出文字说明或离线 mockup。成功标准是 designer 能使用 `playwright-cli` 获取 baseline screenshot，复用固定测试环境，并把 baseline、原型图和状态截图作为可追溯资产放入设计目录。

非目标：

- 不新增 root-worker runtime 功能。
- 不新增 Playwright npm 依赖。
- 不实现独立截图脚本；先通过 agent 工作流约束使用已有 `playwright-cli` skill。

## 技术设计

`ui-ue-designer` frontmatter 将 `skills` 从 `imagegen` 扩展为 `imagegen` 和 `playwright-cli`，让 designer 可以在设计任务中使用浏览器或 Electron 自动化能力获取截图。

涉及现有 root-worker prototype 客户端 UI 时，designer 工作流新增 baseline screenshot 阶段：

1. 复用固定测试环境启动或连接客户端。
2. 用 `playwright-cli` 驱动 Electron 或可连接 Electron 的 Playwright 自动化获取当前界面截图。
3. 将截图保存到 `ui-design/<project-slug>/assets/baseline-*.png`。
4. 在设计文档中引用截图，并基于真实界面说明问题、约束和改造方向。

固定环境路径：

```text
CODEX_HOME=/tmp/my-codex-root-worker-ui-env/codex-home
ROOT_WORKER_WORKSPACE=/tmp/my-codex-root-worker-ui-env/workspace
```

root-worker prototype 当前 Electron app-server client 实际读取 `CODEX_HOME` 并传给 app-server，workspace 默认读取 `ROOT_WORKER_WORKSPACE`，否则落到 `CODEX_HOME/root_workspace`。因此截图工作流以 `CODEX_HOME` 为准，不再建议每次创建新的 `/tmp/root-worker-ui-<slug>` 或类似临时 home。

## 风险

Electron preload 才提供完整 `window.codexDesktop`，普通浏览器打开 Vite 页面可能无法代表真实客户端。工作流要求优先驱动 Electron 或可连接 Electron 的 Playwright 自动化；只有自动化不可用时才使用 Computer Use fallback，并在设计文档记录原因。

固定测试环境会保留历史线程和 workspace 状态。设计任务如果需要干净状态，应在固定环境内部清理目标 workspace 或创建可命名的测试线程，而不是更换 `CODEX_HOME` 根目录。

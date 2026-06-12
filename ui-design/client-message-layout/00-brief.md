# client-message-layout 设计 brief

## 产品目标

root-worker prototype 的会话区需要更像一个可持续阅读的工作线程：连续 `agentMessage` / assistant 输出合并为一个左侧 cell，用户输入右对齐为独立 cell，让“我说了什么”和“系统连续做了什么”在视觉上立即可分辨。

## 目标用户

- 使用者：Codex / root-worker prototype 的开发者、PM、设计和调试人员。
- 使用频率：高频阅读长线程、回看工具执行、调试 live update。
- 设备：桌面为主，移动窄屏用于临时查看和演示。
- 专业程度：能理解 agent、tool、event-command、subagent 等概念，但需要快速扫读。

## 范围

涉及页面和组件：

- `apps/root-worker-prototype/src/lib/conversation.ts` 的 `ConversationEntry -> ConversationCell` 聚合规则。
- `apps/root-worker-prototype/src/components/Conversation.tsx` 的 `MessageRow` 展示。
- `apps/root-worker-prototype/src/components/ConversationVirtualList.tsx` 的 cell 渲染和高度估算影响。
- `apps/root-worker-prototype/src/styles.css` 的 `.message-*`、`.conversation-*` 相关样式。

非目标：

- 不改变 `ThreadItem` typed payload。
- 不从 raw marker、assistant 文本或 JSON envelope 反解展示项。
- 不重新设计 tool/event/compact/archive 的信息结构，只定义它们与 message cell 的分隔边界。

## 约束

- 展示 canonical source 必须保持为 typed `ThreadItem` / v2 payload。
- live 模式下不能因布局合并触发 `thread/read` 或 snapshot merge。
- virtual list 已依赖 row gap、overscan 和高度估算，cell 内合并会改变测量结果，开发时必须检查滚动稳定性。
- 当前卡片圆角建议不超过 8px；既有 prototype 存在 16px radius，本次 handoff 建议逐步收敛到 8px。

## baseline 截图

固定环境：

```text
CODEX_HOME=/tmp/my-codex-root-worker-ui-env/codex-home
ROOT_WORKER_WORKSPACE=/tmp/my-codex-root-worker-ui-env/workspace
```

截图尝试：

1. 使用 `pnpm --filter @my-codex/root-worker-prototype dev` 启动 Electron，Vite 可启动，但 Electron 报错 `Electron failed to install correctly`，无可见 Electron 窗口。
2. 重跑 `node node_modules/electron/install.js` 后再次启动，错误一致。
3. 因 Electron 窗口不可见，Computer Use / macOS `screencapture` 无法获取真实客户端窗口。
4. fallback 使用 `playwright-cli` 打开 Vite renderer 并截图，保存为 [baseline-renderer-fallback.png](assets/baseline-renderer-fallback.png)。该截图为空白页面，说明普通浏览器缺少 Electron preload / app-server 运行环境，不能代表真实 UI。

## 原型资产

- 视觉 handoff mock：[prototype-message-cell-layout.png](assets/prototype-message-cell-layout.png)

## 验收标准

- 用户消息在 LTR 下右对齐，独立成 cell；RTL 后续需另行确认是否镜像。
- 连续 assistant / agent message 合并为一个左侧 cell，内部保留 message 边界、状态和时间信息。
- 合并不能吞掉 pending、error、streaming、attachments、tool/event/compact/archive 的语义边界。
- 桌面和移动宽度下文本不溢出、不与 header/meta 重叠，长代码块可水平滚动或按现有 Markdown 规则处理。
- 开发实现不新增 raw response item 展示分支。
- 当前 fallback 空白截图只证明已尝试自动化 baseline 路径，不可作为视觉验收依据。实现前或实现 PR 中需要在 Electron 可用后重新采集真实 baseline 和 after 截图，再确认布局差异。

# 模型与 reasoning 切换

## 任务 brief

用户需要在 `apps/root-worker-prototype` 当前 thread 中查看并切换后续消息使用的模型和 reasoning effort。成功标准是 header 可打开运行配置入口，模型列表来自 app-server v2 `model/list`，reasoning 选项跟随模型能力，应用后下一次 `turn/start` 传入 `model` 和 `effort`。

非目标：不修改 TUI，不新增 app-server v1 API，不做全局设置页重构，不实现“设为默认”。

## 技术设计

首版只使用现有 app-server v2 能力：

- Electron main 新增 `codex:listModels` IPC，调用 `model/list`，默认不包含隐藏模型。
- app-server v2 `model/list` 在 catalog 列表之外，会读取当前有效配置；如果 `config.model` 对应条目不在 catalog 中，则追加一个可见的 configured model。若该模型已在 catalog 中，则复用现有条目，避免重复。
- configured model 使用 `configured:<provider>:<model>` synthetic id，并在描述中带 provider 名；root-worker 用于展示 `Configured` 标记和 provider 来源，不扩展 v2 协议字段。
- Electron main 新增 `codex:setThreadRunConfig` IPC，只更新当前 prototype session 的 thread runtime cache，不写入用户默认配置。
- Renderer 新增运行配置选择器，挂在 conversation header 原有 model/reasoning chip 位置。
- App 维护当前 thread 的本地 runtime override。应用选择后先同步 Electron main runtime cache，再更新 thread 的 `model` 与 `reasoningEffort`，用于 header 显示和后续发送。
- `sendMessage` payload 增加 `model` 与 `effort`，Electron main 在 `turn/start` 中透传；`turn/steer` 不带 override，避免改变正在运行 turn。
- 选择新模型时，如果当前 effort 不在 `supportedReasoningEfforts` 中，自动 fallback 到该模型 `defaultReasoningEffort`。
- 打开 picker 时，如果当前 thread 的模型不在已加载列表中，不自动选择默认 catalog 模型；只有用户主动选择模型后才更新 draft，避免覆盖当前配置的 custom model。
- 当前 thread 有运行中 turn 或正在发送时禁用应用切换；模型列表加载失败时保留当前 thread 状态，并允许重试。
- Popover 支持点击外部、按 Escape、点击取消关闭；关闭后焦点回到运行配置 trigger，Tab 焦点留在 popover 内。

## 状态与数据流

1. 用户点击 header 的运行配置 chip。
2. picker 调用 `window.codexDesktop.listModels()` 加载模型列表。
3. 用户选择模型，picker 根据该模型支持的 effort 刷新 reasoning 选项。
4. 用户点击应用后，App 调用 `setThreadRunConfig` 更新 Electron main 的 runtime cache，再更新本地 thread runtime。
5. 下一次发送消息时，App 将当前 thread 的 `model`、`reasoningEffort` 传给 preload/main。
6. main 调用 `turn/start` 时加入 `model` 和 `effort`，后端负责让当前 turn 及后续 turn 使用该配置。
7. 若应用配置后、发送前发生 `thread/read` 或 `thread/resume`，main 使用 runtime cache 保留用户选择，避免旧 snapshot 覆盖 header。

## 风险

- app-server 的 `model/list` 失败时无法展示可选模型，需在 popover 内显示错误并提供重试。
- 当前选择只作用于 thread runtime，不写入用户默认配置；后续如要“设为默认”，需单独接入 `config/value/write` 或 `config/batchWrite` 并明确配置 key。
- 后端 `turn/start` 响应或 thread snapshot 的 runtime 字段可能延迟更新，因此 renderer 采用本地状态立即反映用户选择。
- 本设计不提供跨 provider 的 picker 切换。`turn/start` 仍只传 `model` 和 `effort`，provider 维度由当前 session/config 保持；如果后续要在 picker 中切换 provider，需要扩展 `TurnStartParams` 和 core turn 配置派生。

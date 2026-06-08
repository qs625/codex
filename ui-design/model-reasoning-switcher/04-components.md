# 组件拆分与开发 Handoff

## RunConfigTrigger

位置：替换当前 header 中 model / reasoning chip。

Props：

- `modelLabel: string`
- `reasoningLabel: string`
- `isRunning: boolean`
- `isOpen: boolean`

行为：

- 点击打开/关闭 popover。
- 文案格式：`运行配置` + `{modelLabel} · {reasoningLabel}`。
- `aria-haspopup="dialog"`，`aria-expanded` 跟随打开状态。

## RunConfigPopover

Props：

- `threadId: string`
- `currentModel: string | null`
- `currentReasoningEffort: string | null`
- `isTurnRunning: boolean`
- `models: Model[]`
- `modelsStatus: "idle" | "loading" | "ready" | "error"`
- `onRetryModels(): void`
- `onApply(nextConfig): void`
- `onCancel(): void`

行为：

- 打开时创建草稿值，不直接修改 thread。
- 关闭或取消时丢弃草稿。
- thread 切换时自动关闭。
- `model/list` 错误不覆盖当前 thread 配置。

## ModelList

数据来源：app-server v2 `model/list`。

每项展示：

- `displayName`
- `defaultReasoningEffort`
- `description`，若过长则一行截断。

状态：

- Loading：`正在加载模型...`
- Ready：radio list。
- Error：`模型列表加载失败，当前配置未受影响。` + `重试`
- Empty：`暂无可用模型，当前配置未受影响。`

## ReasoningSelector

输入：当前草稿 model 的 `supportedReasoningEfforts` 与 `defaultReasoningEffort`。

规则：

- 只启用 supported efforts。
- 若设计选择“只展示支持项”，隐藏不支持项；若保留全部项，不支持项必须 disabled 并说明原因。
- 推荐第一版只展示支持项，降低密度与解释成本。
- 当前 effort 不受支持时，自动选择 `defaultReasoningEffort`。

## FooterActions

按钮：

- `取消`
- `应用`

禁用规则：

- 没有改动：禁用 `应用`。
- `isTurnRunning === true`：禁用 `应用`。
- `modelsStatus === "error"` 且草稿依赖未知 model：禁用 `应用`。

状态文案：

- Running：`当前 turn 正在运行，结束后可应用切换`
- Fallback：`已回退到该模型默认 reasoning`
- Error：`模型列表加载失败，当前配置未受影响`

## 文案清单

- 入口：`运行配置`
- Scope：`当前 thread`
- 作用域说明：`更改后仅影响当前 thread 的后续消息`
- Model label：`模型`
- Model helper：`来自 model/list`
- Reasoning label：`Reasoning`
- Reasoning helper：`随所选模型支持项变化`
- Loading：`正在加载模型...`
- Error：`模型列表加载失败，当前配置未受影响。`
- Retry：`重试`
- Empty：`暂无可用模型，当前配置未受影响。`
- Running：`当前 turn 正在运行，结束后可应用切换`
- Fallback：`已回退到该模型默认 reasoning`
- Cancel：`取消`
- Apply：`应用`

## 可访问性注意点

- Trigger 使用 button，不用 div 模拟。
- Popover 使用 `role="dialog"` 或语义等价实现，并设置可读 label。
- Model 与 reasoning 使用 radio group 语义。
- 禁用原因必须有文本，不只依赖 disabled 样式。
- 焦点打开后进入 popover，关闭后回到 trigger。
- Escape 关闭，Tab 不应跳到被遮挡区域。
- 颜色对比至少满足 WCAG AA；状态信息不能只靠蓝/红表达。

## 开发入口建议

- UI 修改：基于现有 `apps/root-worker-prototype/src/components/RunConfigPicker.tsx` 补齐中文文案、radio model list、loading/empty/fallback/error 状态和关闭行为。
- Header 挂载：`apps/root-worker-prototype/src/components/Panels.tsx` 已挂载 `RunConfigPicker`，后续主要检查入口位置与禁用条件。
- 样式修改：`apps/root-worker-prototype/src/styles.css` 中 `.run-config-*` 样式需向原型图靠拢，保持浅色工具 UI、紧凑间距、较小圆角和 header 高度稳定。
- 当前 label helper：`apps/root-worker-prototype/src/lib/thread.ts`
- 类型参考：`apps/root-worker-prototype/src/types.ts`
- Electron IPC：`apps/root-worker-prototype/electron/main.cjs`、`apps/root-worker-prototype/electron/preload.cjs` 已有 `codex:listModels`，开发需验证错误恢复、类型映射和空列表处理。
- app-server 客户端：`apps/root-worker-prototype/electron/appServerClient.cjs`
- 协议类型：`codex-rs/app-server-protocol/src/protocol/v2/model.rs`

## 工程风险

- 需要确认现有 `RunConfigPicker` 的 select 控件是否改为 model radio list；设计推荐 radio list，避免隐藏默认 effort 和错误状态。
- 需要确认“应用切换”最终通过现有 `codex:setThreadRunConfig` / 下一次 `turn/start` override 生效；UI 不应只更新 renderer 本地状态。
- Running 判定必须来自可靠 thread/turn 状态，不能只看最后一条消息文本。
- 当前初版实现仍有英文文案，应全部替换为本文档文案清单。
- Popover 应支持 outside click、Escape 关闭和关闭后焦点返回 trigger。
- 如果未来支持 provider 分组，ModelList 应保留分组扩展空间。

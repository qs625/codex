# 运行配置切换 UI/UE Handoff

## 设计结论

- 入口放在 conversation header 原有模型与 reasoning chip 位置，合并为一个可点击的“运行配置”入口。
- Popover 是紧凑工具控件，不进入 Settings，也不占用右侧面板；作用域文案明确为“更改后仅影响当前 thread 的后续消息”。
- 模型选择来自 app-server `model/list`。Reasoning 选项跟随当前模型的 `supportedReasoningEfforts`，不使用固定枚举。
- 当前 turn 运行中或正在发送时不允许应用配置，popover 内显示禁用原因。
- 模型列表加载失败只影响 popover，保留当前 thread 的现有配置，并提供“重试”。
- “设为默认”不进入首版，避免在未确认 config write 语义前误导用户。

## 交互状态

- 默认：trigger 显示当前模型和 reasoning，例如 `运行配置 · gpt-5.5 · medium`。
- 打开：popover 显示模型控件、Reasoning 控件、模型描述、“取消”和“应用”。
- 切换模型：如果原 reasoning 不被新模型支持，自动选择新模型的 default reasoning。
- 无变化：“应用”禁用。
- 运行中：选择可查看，“应用”禁用，并显示 `当前 turn 正在运行，结束后可应用切换`。
- 加载失败：显示错误和“重试”，当前 header 不变。

## 文案

- 标题：`运行配置`
- 作用域说明：`更改后仅影响当前 thread 的后续消息`
- 运行中提示：`当前 turn 正在运行，结束后可应用切换`
- 错误提示：`模型列表加载失败，当前配置未受影响。`
- 操作：`取消`、`应用`、`重试`

## 可访问性

- Trigger 使用 `aria-haspopup="dialog"` 与 `aria-expanded`。
- Popover 使用 `role="dialog"` 和明确 aria label。
- Reasoning 选项使用 `radiogroup` / `radio`，当前项使用 `aria-checked`。
- 触发器和操作按钮保持原生 button/select 控件，支持键盘聚焦。
- 点击外部、按 Escape、点击“取消”都应关闭 popover；关闭后焦点回到 trigger。

## 开发 handoff

- 保持 header 高度稳定，trigger 文本过长时截断，不挤压 cwd chip。
- Popover 宽度不超过 360px，并限制在 viewport 内。
- 当前 worktree 已有 `RunConfigPicker`、`codex:listModels`、`codex:setThreadRunConfig` 初版实现；开发应基于现有组件补齐中文文案、状态矩阵、radio model list、fallback 提示和可访问性关闭路径。
- App 层应用选择后先同步 Electron main 的 runtime cache，再更新 renderer thread state。
- 真正后端生效点仍然是下一次 `turn/start` 的 `model` 和 `effort` override。

## 原型资产

- `assets/baseline-current-app-clean.png`：当前应用真实 baseline。
- `assets/run-config-current-app-modification.png`：基于当前应用截图的功能修改后局部原型图。
- `assets/run-config-component-states.png`：只包含运行配置相关组件的状态图。

## 剩余 UX 风险

- 首版不写默认配置；如果用户预期跨 thread 生效，需要在后续版本加入独立默认设置入口。
- 组件状态图未单独画 loading、empty、fallback 小态；这些状态以 `02-ue-flow.md` 和 `04-components.md` 为实现真源。

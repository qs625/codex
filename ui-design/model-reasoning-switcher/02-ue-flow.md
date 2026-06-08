# UE 交互流程

## 主路径

1. 用户在 thread header 看到“运行配置”入口，入口内显示当前摘要：`GPT-5.4 · High`。
2. 用户点击入口，打开右上对齐的 popover。
3. Popover 顶部显示标题“运行配置”和作用域说明：“更改后仅影响当前 thread 的后续消息”。
4. 客户端请求 `model/list`。加载完成后展示 model 列表。
5. 用户选择一个 model。
6. Reasoning 区域立即更新为该 model 支持的 efforts。
7. 如果当前 effort 仍受支持，保持选择；如果不支持，选择该 model 的 default effort，并显示短提示。
8. 用户点击“应用”，popover 关闭，header 摘要更新；后续发送消息使用新配置。

## 运行中状态

- 当前 turn 正在运行时，用户仍可打开 popover 查看配置。
- Model 与 reasoning 可保持可浏览，但“应用”按钮禁用。
- 底部显示：“当前 turn 正在运行，结束后可应用切换。”
- 如用户改变候选选择，界面可保留草稿选择，但不提交；运行结束后按钮恢复。

## model/list 加载

- 打开 popover 后若无缓存，model 区域显示小型 inline loading：“正在加载模型...”
- 已有缓存时先展示缓存，同时可在角落显示“正在刷新...”。
- 加载不阻塞查看当前配置。

## model/list 错误

- 错误态保留当前 thread 配置摘要。
- Model 区域显示：“模型列表加载失败，当前配置未受影响。”
- 提供“重试”按钮。
- Reasoning 区域仍可展示当前 thread 的 effort，但不可提交未知 model 切换。

## 空状态

- 若 `model/list` 返回空数组，显示：“暂无可用模型，当前配置未受影响。”
- “应用”按钮禁用。
- 入口仍显示当前 thread 已有摘要。

## Effort fallback

触发条件：用户选择的新 model 不包含当前候选 effort。

行为：

- Reasoning 自动切换为该 model 的 `defaultReasoningEffort`。
- 在 reasoning label 右侧显示一条临时提示：“已回退到该模型默认 reasoning。”
- 提示不超过一行，避免扩大 popover。

## 关闭与取消

- 点击“取消”、按 Escape、点击 popover 外部均关闭并丢弃未应用草稿。
- 若无改动，“应用”按钮禁用。
- 若切换 thread，popover 自动关闭，避免把配置应用到错误 thread。

## 键盘流程

- 入口按钮获得焦点后按 Enter / Space 打开。
- Popover 内 Tab 顺序：model 列表、reasoning 选项、取消、应用。
- 单选列表支持方向键移动选择。
- Escape 关闭并回到入口按钮。

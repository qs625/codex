# UE 交互流程

## 主路径：选择 catalog model

1. 用户点击 header 的 `运行配置` trigger。
2. Popover 打开，草稿值等于当前 thread 的 `model` 与 `reasoningEffort`。
3. 系统加载并展示 catalog models，同时合并 configured custom models。
4. 用户选择一个 catalog model。
5. Reasoning 选项按该 model 的 `supportedReasoningEfforts` 更新；若原 effort 不支持，草稿回退到该 model 的默认 effort，并显示 `已回退到该模型默认 reasoning`。
6. 用户点击 `应用`。
7. Popover 关闭，后续消息使用新配置。

## 主路径：选择 configured custom model

1. 用户打开 picker。
2. 自定义模型出现在 catalog 分组之后，模型名为 `displayName || model`，meta 显示 `Configured · 默认 {effort}`，右侧或 meta 行显示 provider 名。
3. 每个 configured custom model 带中性 `Configured` 标记。
4. 用户选择 custom model 后，Reasoning 根据该 configured model 的支持项更新。
5. 点击 `应用` 后才更新当前 thread 配置。

## 当前 model 不在 catalog

1. 用户打开 picker。
2. 如果当前 thread 的 `model` 不在 catalog，也不在 configured custom models 中，列表首项显示一个 synthetic current item。
3. 文案：
   - 标题：当前 `model`
   - Meta：`当前配置 · 未出现在 model/list`
   - 辅助说明：`继续显示；只有用户选择其它项才改变。`
4. 该项默认选中，打开 picker 不自动切换到 catalog 默认模型。
5. 如果用户直接关闭或取消，当前 thread 配置完全不变。
6. 如果用户选择其它项并应用，才替换未知当前模型。

## 加载状态

- 首次打开时显示 `正在加载模型...`。
- 如果已有当前 thread 配置，trigger 继续显示当前值，不被加载状态覆盖。
- 加载期间可以关闭 picker；关闭不取消当前配置。

## 错误状态

- 文案：`模型列表加载失败，当前配置未受影响。`
- 操作：`重试`。
- 若当前配置可构造 synthetic current item，保留当前项可见；否则只展示错误和 actions。
- `应用` 禁用，除非用户已经有可验证的草稿选择。

## 空状态

- 文案：`暂无可用模型，当前配置未受影响。`
- 若当前 model 存在，仍显示 synthetic current item；没有可选新模型时 `应用` 禁用。

## Running turn

- 用户可以打开 picker 查看当前配置和可选项。
- `应用` 禁用。
- 状态文案：`当前 turn 正在运行，结束后可应用切换`。
- running 结束后无需强制关闭 picker；按钮可恢复启用，但草稿仍以用户当前选择为准。

## 反馈与关闭

- `取消`、点击外部、Escape：关闭并丢弃草稿。
- thread 切换：关闭 picker，避免把 A thread 草稿应用到 B thread。
- 应用失败：回滚本地 optimistic update，并显示全局错误；picker 可重新打开重试。

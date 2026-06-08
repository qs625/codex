# 信息架构

## Header

Header 右侧保留一个主要配置入口：

- 图标：settings / sliders 类图标。
- 主文案：`运行配置`。
- 摘要：`{modelDisplayName} · {Reasoning}`。
- 状态：运行中时可显示轻量 busy 指示，但不改变入口位置。

不再并列展示独立 model chip 与 reasoning chip，避免两个入口看起来可分别操作。

## Popover 结构

1. 标题区
   - 标题：`运行配置`
   - Scope badge：`当前 thread`
   - 说明：`更改后仅影响当前 thread 的后续消息`

2. Model 区
   - Label：`模型`
   - 辅助信息：`来自 model/list`
   - 列表项：模型名、默认 reasoning、可选短描述

3. Reasoning 区
   - Label：`Reasoning`
   - 辅助信息：`随所选模型支持项变化`
   - 选项：按 model 支持项展示 `Low / Medium / High / XHigh` 等

4. 状态区
   - Running：`当前 turn 正在运行，结束后可应用切换`
   - Fallback：`已回退到该模型默认 reasoning`
   - Error：`模型列表加载失败，当前配置未受影响`

5. 操作区
   - `取消`
   - `应用`

## 响应式策略

- 桌面：popover 宽度 340-380px，右上对齐 header 入口。
- 窄屏：popover 宽度为视口宽度减左右 16px，仍从入口下方展开。
- 列表高度受限时，model 区内部滚动，footer 固定在底部。
- 不使用全屏 modal，除非未来移动端需要单独设计。

## 信息优先级

1. 当前作用域与当前值。
2. 可选 model。
3. 该 model 支持的 reasoning。
4. 运行中/错误/fallback 状态。
5. 操作按钮。

# Information Architecture

## 全局结构

- 中央 conversation：线程事实记录，包括 user、assistant、tool、event、schedule、command notification 等 typed entries。
- 右侧 Thread Analysis：状态索引和上下文摘要，包含 Command Activity / Live Commands。
- command cell：conversation 内 command session 的完整详情入口。

## Command Session 信息层级

Command cell header：
- 主文本：command，单行截断，title 保留完整命令。
- 次级文本：cwd path、status、duration 或 running time。
- 状态 pill：`Running`、`Waiting`、`Completed`、`Exit N`、`Failed`。

Command cell details：
- Execution：command、cwd、status、duration、exit code。
- Session：initial wait、notify on、initial/yield wait alias、tty、max output tokens、sandbox/approval 参数。没有 typed 字段时显示 `Not provided`，但开发优先补 typed 数据契约。
- Output summary：最后非空输出行、输出截断说明、是否仍有 live tail。

Notification event：
- 位置：conversation 中作为独立 event/tool-adjacent entry 出现，不塞进 command cell 的 details。
- 主文案：`Command output notification` 或 `Command exit notification`。
- 次级文案：关联 command 的短命令、cwd、notification 触发原因、收到时间。
- 详情：output chunk 摘要或 exit code / duration / notification source。

RightPanel Live Commands：
- 只保留 active/recent-attention 列表。
- 每行显示 command、cwd、状态、latest notification 或 latest output tail。
- 不显示完整 output，不替代 conversation details。

## Composer Slash 菜单信息层级

位置：菜单锚定在底部 composer 上方，最大宽度跟随 composer，最大高度不超过 conversation 可视区的 40%；在小窗口中向上展开并内部滚动。

层级：
- 输入区：保留 composer draft、selection 和已有 Skill chips。
- 弹层 header 不常驻显示；分组标题直接承担结构说明。
- `Commands` 分组优先展示高频内置命令，候选行包含 command token、动作说明、执行提示。
- `Skills` 分组展示可用 skill，候选行至少包含 `$skill-name`；说明、来源和可用状态仅在现有数据或后续 metadata 可用时展示。
- 状态行只出现在对应分组内：loading、error、empty，不占用 active 候选序列。

响应式：
- 桌面宽度充足时菜单左边缘与 composer 输入文本起点对齐。
- 窄窗口时菜单左右贴合 composer 容器内边距，不覆盖发送按钮和关键状态提示。
- 长候选名单行截断，完整名称进入 `title` / tooltip；说明最多一行。

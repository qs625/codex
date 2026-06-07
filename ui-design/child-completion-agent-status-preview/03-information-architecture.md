# 信息架构

## Conversation 列表层

列表层只承载扫描信息，字段优先级如下：

1. 事件类型：Agent Status / child completion。
2. 状态：completed、errored、interrupted、shutdown、not found、running、pending init。
3. agent label：优先 nickname，其次 path，最后 thread id fallback。
4. role：有值时以 `[worker]` 形式附在 label 后。
5. message preview：只作为状态补充，不参与标题主语。

默认最大高度：

- 标题行：1 行，必选。
- 摘要详情：最多 1 行，message 存在时显示。
- 辅助元数据：默认不在列表显示，除非没有可读 agent label。

因此默认 item 高度建议为 1 到 2 行，硬上限为 3 行。

## Details / 展开层

details 层承载完整信息，字段顺序如下：

1. 状态摘要：status + agent label。
2. 路径和线程元数据：`senderPath`、`recipientPath`、`senderThreadId`、`recipientThreadId`。
3. 完整 `status.message`。
4. 原始 item id 或 debug metadata。

完整 message 不做截断，但需要滚动容器或可折叠区域限制总体高度。

## 响应式策略

宽屏：

- 标题行保留 status + agent label + role。
- 第二行展示 message preview。
- 不在列表中展示完整 path，避免重复和噪音。

窄屏：

- 标题行优先保留 status + nickname。
- role 可保留，长 path 必须 middle truncate。
- message preview 独占第二行，按可用宽度截断。
- 不允许横向滚动撑开 conversation。

极窄宽度：

- 标题行可降级为 `Completed Robie` / `Error Robie`。
- 详情行继续单行截断。
- 完整 path 只在 details 中显示。

## 信息层级示例

短 completion：

```text
• Completed Robie [worker]
  └ Added focused regression coverage
```

长 completion：

```text
• Completed Robie [worker]
  └ Implemented the renderer state redirect, preserved terminal...
```

错误：

```text
• Error Robie [worker]
  └ cargo test failed in codex-tui snapshot comparison...
```

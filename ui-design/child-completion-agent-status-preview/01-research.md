# 相关模式调研

## 内部现状

当前 TUI multi-agent history cell 使用“标题行 + 可选详情行”的结构。`SpawnAgent`、`SendInput`、`Wait`、`ResumeAgent` 都把关键动作放在标题行，次要信息放在 `└` 缩进详情中。这种结构适合 conversation 中快速扫描。

现有代码已经定义了三个 preview 长度常量：

- prompt preview：160 graphemes。
- error preview：160 graphemes。
- response preview：240 graphemes。

`Completed` status 目前会把 `status.message` 先做 whitespace collapse，再通过 `truncate_text` 截断。这说明产品已经接受“列表中只放 completion preview，完整内容另处查看”的方向。

## 问题归因

child completion / Agent Status item 的主要信息是状态事件，而不是最终回答正文。把完整 completion 直接放入列表，会带来三个问题：

- 扫描成本高：用户需要看到多个 agent 的完成顺序和状态，长正文会打断时间线。
- 视觉高度失控：一个长 completion 会挤掉上下文，尤其在窄面板中更明显。
- 信息层级反转：状态、agent 身份和错误信号被正文淹没。

## 同类设计模式

适合采用“事件摘要 + 详情保留”的模式：

- 列表行只展示事件身份：状态、agent label、短 message。
- 长正文进入详情区、展开区或 inspector。
- 错误状态优先暴露错误摘要，避免用户必须展开才能发现失败原因。
- 重复终态事件保留，运行中状态可以合并或刷新，避免噪音。

## 设计原则

- 默认视图服务扫描，不服务完整阅读。
- 摘要必须可预测：同一类 item 的高度和字段顺序稳定。
- 详情必须无损：截断只发生在列表摘要，不影响完整 completion 数据。
- 状态优先于正文：`Completed`、`Errored`、`Interrupted` 等状态标签永远出现在长文本之前。

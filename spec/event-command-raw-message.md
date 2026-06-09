# EventCommand raw message 泄漏修复

## 任务 brief

用户在订阅 `event_command_subscribe` 后，客户端偶尔看到后台事件的 raw JSON 或
`<event_command>{json}</event_command>` marker 被当成普通消息展示，例如 stdout 行的
`subscription_id` / `line` JSON 直接出现在消息流里。

成功标准：

- `ResponseItem::EventCommandEvent` 在 app-server 默认和 raw events 开启路径中都只产生结构化
  `eventCommandEvent` item，不再额外产生普通 raw message。
- 历史或模型 echo 里的 `<event_command>{json}</event_command>` marker 也按同一规则结构化，不作为普通
  assistant/user message 残留。
- `EventDrivenTool` marker 使用相同边界规则，避免同类 event item 泄漏。
- 普通 assistant 文本、普通 JSON 文本和仍未结构化的 raw response item 不受影响。
- 不改变 event command 进入模型上下文的语义。

非目标：

- 不在 root-worker 客户端增加 raw JSON 白名单或显示层过滤。
- 不修改 TUI。
- 不改变 event command registry、history 记录或 provider 请求前文本化逻辑。
- 不关闭 `experimentalRawEvents` 对普通 raw response item 的调试能力。

## 技术设计

`codex-core` 会把 `EventCommandEvent` 记录为 typed `ResponseItem`，并在发给模型前通过
`EventCommandEvent::to_response_item()` 文本化为 marker message。这个语义需要保留，因为模型必须能收到后台
事件。

app-server 已经在 `RawResponseItem` 分支把 typed `EventCommandEvent`、typed `EventDrivenTool`、以及它们的
marker message 翻译成结构化 `item/completed`。泄漏来自同一分支随后继续发送
`rawResponseItem/completed`，客户端在 raw events 开启时可能把它当普通消息消费。

最小修复是在 app-server 协议边界增加判定：如果一个 raw response item 已经由结构化 app-server item 表示，
则不再发送 `rawResponseItem/completed`。该规则只命中：

- `ResponseItem::EventCommandEvent`
- `ResponseItem::EventDrivenTool`
- 可解析为 event command marker 的 `ResponseItem::Message`
- 可解析为 event driven tool marker 的 `ResponseItem::Message`

普通 message、普通 JSON、其他 response item 仍保留 raw events 行为。

## 风险

主要风险是 `experimentalRawEvents` 使用者原本依赖这些内部 event marker 的 raw 副本。该副本与结构化
`item/completed` 表示同一事件，保留会污染普通消息流；抑制只影响已成功结构化翻译的内部 item，不影响其他 raw
调试数据。

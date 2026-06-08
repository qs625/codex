# Chat Completions Provider 适配

## 任务 brief

用户需要通过 `config.toml` 配置非 Responses API 的模型服务，目标服务兼容 Azure OpenAI Chat Completions 请求形态，支持普通 assistant 文本和 function tool calls。成功标准是 provider 层可以按新的 `wire_api` 发送 chat-completions 请求，并把响应映射回 Codex 现有 `ResponseEvent` / `ResponseItem` 流程，不影响 Responses API provider。

非目标：

- 不改 TUI、root-worker 或客户端 UI。
- 不移除 Responses API provider 支持。
- 不提交任何真实 API key、token、Authorization header 或本机私密配置。
- 不实现 Responses-only 能力在 chat-completions provider 上的完整等价语义。

## 配置形态

新增两个 provider wire API：

- `wire_api = "chat_completions"`：`base_url` 表示 OpenAI-compatible API base，客户端会请求相对路径 `chat/completions`。例如 `https://api.example.com/v1` 会请求 `https://api.example.com/v1/chat/completions`。
- `wire_api = "azure_chat_completions"`：`base_url` 表示完整 Chat Completions endpoint，客户端不会再追加 `/responses` 或 `/chat/completions`。适合 Azure deployment endpoint 或公司内网已经给出完整 chat-completions 入口的服务。

两种形态都继续复用现有 `query_params`、`http_headers`、`env_http_headers`、`env_key`、`auth`、retry 和 timeout 配置。`api-version` 应通过 `[model_providers.<id>.query_params]` 配置。

示例：

```toml
model_provider = "modelhub-gpt"
model = "gpt-5.5-2026-04-24"

[model_providers.modelhub-gpt]
name = "ModelHub GPT"
base_url = "https://example.com/api/modelhub/online/v2/crawl"
wire_api = "azure_chat_completions"
env_key = "MODELHUB_API_KEY"

[model_providers.modelhub-gpt.query_params]
api-version = "2024-03-01-preview"

[model_providers.modelhub-gpt.env_http_headers]
X-TT-LOGID = "MODELHUB_LOGID"
```

## 技术设计

`core` 继续先构建 canonical `ResponsesApiRequest`，这样 prompt、工具列表、reasoning/text 控制和现有 tracing 入口不需要重复实现。新增 `ChatCompletionsClient` 接收该请求并在 provider 边界做 wire format 转换。

请求映射：

- `instructions` 映射为第一条 `system` message。
- `ResponseItem::Message` 映射为 chat message；`developer` 角色降级为 `system`，避免 Azure chat-completions 不识别。
- `ResponseItem::FunctionCall` 映射为 assistant message 的 `tool_calls`。
- `ResponseItem::FunctionCallOutput` 映射为 `role = "tool"` 的 tool result message。
- Responses function tools 从 `{ type: "function", name, description, parameters, strict }` 转为 chat-completions 的 `{ type: "function", function: { ... } }`。
- 没有可发送的 function tools 时省略 `tools`、`tool_choice` 和 `parallel_tool_calls`，避免兼容服务拒绝无工具请求。
- Responses-only 工具类型暂不发送给 chat-completions provider，避免构造无效请求。

响应映射：

- 非 streaming chat-completions response 转为有限 `ResponseStream`。
- 非 streaming assistant `content` 映射为完整 `ResponseItem::Message`，事件顺序为 `Created -> OutputItemDone -> Completed`，避免发送没有 active item 的裸 `OutputTextDelta`。
- `tool_calls[].function` 映射为 `ResponseItem::FunctionCall`。
- 旧式 `message.function_call` 也映射为 `ResponseItem::FunctionCall`，缺少 tool call id 时使用稳定 fallback call id `legacy_function_call`。
- `finish_reason = "stop"`、`"tool_calls"` 或旧兼容值 `"function_call"` 视为正常完成；`"length"`、`"content_filter"` 等其他结束原因映射为 stream error，避免把截断或过滤内容当作完整回答。
- 空 `choices` 响应映射为 stream error。
- 最后发送 `ResponseEvent::Completed`，token usage 映射到 `TokenUsage`。

## 风险

- Chat Completions 不支持 Responses 的 reasoning summary、encrypted reasoning、remote compaction、web search/image generation 等能力；本适配只覆盖文本和 function tool calls。
- 部分 Azure-compatible 服务只支持非 streaming。本实现优先用非 streaming `.create()` 形态，仍输出 Codex 内部流事件。
- 如果 provider 依赖完整 Azure deployment path，必须使用 `azure_chat_completions`，否则 `chat_completions` 会追加 `chat/completions`。

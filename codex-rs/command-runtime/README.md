# codex-command-runtime

`codex-command-runtime` 承载不依赖 `codex-core` 的 command runtime primitive：

- command output 的 capped head/tail buffer。
- process exit/failure state。
- command wait 和 write-stdin 的 request/response DTO。
- command notification filter/state。
- yield time、max token 和 chunk id 这类纯 helper。

这个 crate 不负责执行命令，也不处理 approval、sandbox、PTY spawn、async watcher event
emission、`Session` 或 `TurnContext` 编排。这些 runtime glue 继续留在 `codex-core`。

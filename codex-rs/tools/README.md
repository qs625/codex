# codex-tools

`codex-tools` 是兼容 facade crate，只 re-export `codex-tool-planning` 的公开 API。
新的 tool planning、discovery、MCP/dynamic conversion 或内建 tool spec 构造逻辑应放在
`codex-tool-planning`，不要继续添加到这个 facade。

这个 crate 的存在只为下游渐进迁移和旧路径兼容。`codex-core` 不应依赖它。

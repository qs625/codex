# 放宽 Markdown Agent Frontmatter 解析

## 任务 brief

项目内 `.codex/agents/*.md` agent 文件需要兼容更宽松的 frontmatter。成功标准是只有 `description` 仍为必填；`name` 缺失时使用文件名；`tools` 和 `skills` 缺失时默认全部可用；其他已支持字段缺失时沿用既有默认值；未知字段不影响识别。

非目标：

- 不改变 `.toml` agent role 文件解析规则。
- 不改变 agent 文件发现目录、递归策略或扩展名规则。
- 不处理模型 provider 配置。

## 技术设计

Markdown agent metadata 继续使用现有 YAML frontmatter 和 `RawAgentRoleFileMarkdown` 结构体解析，但不再拒绝未知字段，让项目自定义字段可以自然忽略。

role name 的解析顺序：

1. frontmatter `name` 存在且非空时优先使用。
2. 否则使用 Markdown 文件的 file stem，例如 `optimizer.md` 得到 `optimizer`。
3. 文件名无法转换为 UTF-8 或为空时仍报错。

`tools` 和 `skills` 的 Markdown 缺省值从继承态改为 `AgentCapabilityAllowlist::All`，使未声明 allowlist 的 Markdown agent 明确拥有全部可用能力。显式声明的 `tools` / `skills` 仍复用现有 allowlist 解析规则，包括 `"*"` 和 pattern 列表。

`description` 继续在加载与合并阶段通过 `validate_required_agent_role_description` 校验，缺失时 agent 被忽略并写入 startup warning。

## 风险

Markdown agent 文件缺省 `tools` / `skills` 改为显式全部后，不会再从低优先级同名角色继承 allowlist。这与“缺失时默认全部”的需求一致，但属于行为变化，已通过单元测试和 discovery 集成测试锁定。

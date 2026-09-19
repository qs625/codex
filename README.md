# Morpheus

Morpheus 是从 OpenAI Codex fork 出来的本地优先 agent runtime 和桌面工作区。它保留 Codex 的 Rust coding-agent / app-server / MCP / sandbox 基础，再向“长期运行、多 agent 协作、可回放、可安装更新、可桌面调试”的本地 agent 操作系统方向扩展。

这不是单纯把 Codex 改个壳：Morpheus 的核心改动集中在 runtime、thread、agent、typed history、桌面安装态、Computer Use、workflow 和 PM/owner/reviewer 协作流程上。

## 核心功能

- Root Worker 桌面客户端：Electron 桌面 UI，覆盖 conversation、project tree、Terminal/command output、Browser panel、artifact、agent tree 和 self-debug。
- Runtime Capsule：稳定桌面壳 + 可替换 runtime artifact；记录 selected/current/previous release，支持失败回滚、重启恢复和安装态验收。
- Thread / agent runtime：围绕 thread、turn、tool call、event replay、agent path、agent status、goal、compact、subscription 组织，而不是只围绕一次 CLI loop。
- 多 agent 协作：PM / owner / reviewer 长期 agent，native subagent 与 external CLI agent 并存，支持 follow-up、list/read/close、全局虚拟 agent path。
- Typed history：模型上下文、持久化历史、UI 展示、compact marker、artifact、command event、external tool event 都走 typed item，不靠 raw transcript 或 marker 反解析。
- Context / compact：compact checkpoint、segmented rollout、init context、skill/tool context、model-visible history 和 display history 分层处理。
- Computer Use via MCP：稳定 macOS helper app、LaunchServices host、TCC 权限诊断、target-bound visual proof、前后台动作边界。
- 自举开发流程：固定 canonical checkout + 固定 dev checkout，owner 做 focused validation，PM merge 后做完整 Capsule build、restart、self-debug 安装态验证。

## 完整项目对比

下面的对比基于当前本地对照代码树：

- Morpheus：本仓库 `~/.morpheus/source_workspace`
- OpenAI Codex：`~/.morpheus/reference-sources/openai-codex`
- Claude Code Best：`~/.morpheus/reference-sources/claude-code-best-claude-code`
- DeepSeek Harness / dsh：`~/.morpheus/reference-sources/deepseek-ai-deepseek-harness`

| 维度 | Morpheus | OpenAI Codex | Claude Code Best | DeepSeek Harness / dsh |
| --- | --- | --- | --- | --- |
| 代码基底 | Codex fork；Rust `codex-rs/` + TS/Electron `apps/root-worker-prototype/` | Rust `codex-rs/` + `codex-cli/`，官方 Codex CLI / app-server 基底 | Bun/TypeScript 项目，`src/` + `packages/*`，复刻并扩展 Claude Code 体验 | PNPM/TypeScript monorepo，`apps/*` + `packages/*` + `native/system` |
| 产品核心 | 本地桌面 agent runtime/workspace，强调长期运行、typed replay、安装态更新和多 agent 协作 | 本地 coding agent，重点是 CLI、IDE/App 入口、tool loop、sandbox、MCP、app-server | 终端 coding agent 体验增强，强调 Claude Code 兼容、TUI、命令、daemon、workflow、remote control | “everything-is-a-plugin” agent harness，基于 Cordis plugin tree 组合 runtime |
| 主入口 | `apps/root-worker-prototype` 桌面客户端；`codex-rs/app-server` runtime；`/self` 源码项目 | `codex` CLI、Codex app、`codex-rs/app-server` | `ccb` / `ccb-bun` CLI，Bun/Node 双入口 | `dsh web` / `dsh --profile ...`，web/desktop/headless/sdk/acp profiles |
| 架构中心 | ThreadService、AppServer、Runtime Launcher、Root Worker UI、typed EventMsg -> ThreadItem | codex-core、app-server、protocol、tools、MCP、sandbox | CLI/TUI session、commands、services、daemon、workflow-engine、remote-control | Cordis context、插件服务、profile/bundle/patch、agent-loop、session event log |
| 扩展方式 | Morpheus skills/agents/workflows、MCP、external provider、runtime tools；更强调 provider-neutral runtime facts | `.codex` skills/config、MCP、plugins、hooks、CLI/app-server 协议 | slash commands、skills、plugins、MCP、workflow scripts、feature flags | 插件是一等架构单元；profile + bundle + patch 组合整棵 runtime |
| Agent 模型 | native agent 和 external CLI agent 都进入 thread/agent path/runtime 状态；PM/owner/reviewer 是常规协作模式 | 有 agent roles、subagent/thread 相关能力，但产品主线更偏单 agent coding | fork/subagent、Ultracode workflow、pipe/LAN 多实例协作是增强能力 | `core/agent`、`core/agent-loop`、`subagent`、agent lifecycle 是 plugin runtime 的核心包 |
| 状态和历史 | 明确分 live runtime state、persisted replay history、model-visible context；UI 只消费 typed thread item | 有 history、rollout、app-server protocol、MCP/tool event 基础 | 以 CLI session transcript、daemon/job 状态和命令面板为主 | 追加式 session event log，事件和 plugin lifecycle 是架构中心 |
| UI 形态 | 桌面 Root Worker：conversation + project tree + Browser/Terminal/RightPanel + artifacts + debug | CLI / IDE / app 入口；README 主推 CLI 和 Codex app | Ink/TUI 为主，带 remote control、自托管界面、面板命令 | Web UI、desktop app、CLI/headless/SDK 多 profile |
| Computer Use | Morpheus 自有 MCP server + macOS helper app + LaunchServices host + stable permission subject + visual proof | 官方 Codex 代码里有 Computer Use / app integration 相关方向，但不是本 fork 的实现边界 | README/docs 明确有 Computer Use、Chrome Use、MCP 测试/架构文档 | 有 `packages/computer-use` / `browser-use`，作为插件化子系统 |
| Workflow | 项目内 `.morpheus/workflows` + runtime workflow API，长期方向是可恢复状态机和 agent.wait 语义 | 有 workflow 相关 crate/功能，但不是 README 主线 | `/ultracode` / `Workflow` 工具，脚本化 `agent/parallel/pipeline/phase`，带监控面板 | `packages/workflow` 与 Cordis plugin 体系结合 |
| 桌面安装 | 外层 Launcher + Seed Capsule + external Runtime Capsule；安装态和源码 merge 明确分离 | 官方 CLI/app 分发和 app-server 基础 | 主要是 npm/Bun CLI 包与 remote/self-host 能力 | Web/desktop package/profile；desktop host 启动 web app |
| 构建系统 | Rust/Cargo + PNPM/Electron；Codex 基底上进一步拆出 IoC / service / API crate 边界，PM 集成才跑完整 mac Capsule | Rust/Cargo + PNPM/Bazel 辅助；官方 Codex 构建/安装脚本 | Bun build、Vite、native napi packages、npm package | PNPM workspace、TS build、native/system、web/desktop/package scripts |
| 冷/热编译迭代 | 通过 IoC 化和窄 crate/service 边界降低 app-server 对具体实现的重编译耦合；固定 checkout 复用 `target`/`node_modules`，热路径更常停在 focused crate/test/debug build，冷路径集中到 PM Capsule 交付 | 上游通用构建路径；核心 runtime 耦合更接近 Codex 原始结构 | Bun/TS 热迭代轻，但不是 Rust app-server + Capsule 交付模型 | TS/PNPM plugin 架构，构建面广；profile/plugin 热插拔是核心优势 |
| 最适合的问题 | 需要本地桌面常驻、多 agent 协作、runtime 可审计、安装态自更新和 Computer Use 权限稳定的 agent 产品 | 通用本地 coding agent、官方 Codex 生态和 IDE/CLI 集成 | 想要 Claude Code 兼容体验、TUI 增强、workflow/remote/群控扩展 | 想要 plugin-first、profile/patch 组合、web/harness/SDK 可插拔实验 |

## Morpheus 相对 Codex fork 的优势

Morpheus 的优势不是“替代 Codex”，而是在 Codex 基础上把产品重心移到本地 agent runtime：

1. **桌面 runtime 交付闭环**：Morpheus 有 Runtime Capsule、Launcher control state、selected/current/previous release、full restart、self-debug。代码 merge 后还要证明 installed-effective。
2. **多 agent 是默认工作流**：PM/owner/reviewer 固定 checkout 协作不是临时提示词，而是项目日常开发方式；任务状态写入 `.codex/pm-progress.md`，交付后再由 PM 合并和安装态验收。
3. **typed history 和 replay 更强**：Morpheus 不把 UI 展示建立在 assistant 文本 marker 上，而是把 command、artifact、compact、external tool、thread item 等变成 typed durable facts。
4. **更适合长期桌面运行**：Root Worker UI 管 conversation、project、browser、terminal、agent tree、artifact 和 debug；不是只面向一次命令行会话。
5. **Computer Use 权限主体更稳定**：通过独立 helper app + MCP 暴露能力，目标是让 macOS Screen Recording/Accessibility 权限附着在稳定 app 上，而不是随 Runtime Capsule 路径漂移。
6. **IoC 架构带来的冷/热编译收益**：Morpheus 在 Codex 基础上把大量 app-server / thread / tool / MCP / memory / goal / command / plugin / model 等能力拆成窄 service crate、API crate 和 runtime trait 边界，让高层 app-server 更多依赖抽象接口和组合关系，而不是把所有实现细节压在一个大核心里。这样改动局部 service 时，Cargo 的失效范围更小；热编译更容易停在 focused crate/test/debug build，冷编译和 fat-LTO release package 则集中到 PM 集成点执行。

## 与 Claude Code Best 的差异

Claude Code Best 的代码结构明显围绕 Bun/TS CLI/TUI 展开：`src/commands`、`src/components`、`src/daemon`、`src/workflow`、`packages/workflow-engine`、`packages/remote-control-server` 等都服务于增强 Claude Code 终端体验。

Morpheus 与它的主要区别：

- Morpheus 的核心在 Rust app-server/thread runtime + Electron desktop，不是 TUI-first。
- Morpheus 更强调 runtime-owned typed facts、reload/replay 和安装态生命周期。
- Claude Code Best 的 workflow 脚本体验更接近“在 CLI 内跑确定性编排工具”；Morpheus 的长期方向是把 workflow 状态机、owner 空闲、completion gate、agent wait 等变成 runtime 可恢复事实。
- Claude Code Best 的 remote/control、群控、TUI feature 很丰富；Morpheus 更关注本机桌面 Root Worker、自身源码 `/self`、Runtime Capsule 和 provider-neutral native/external agent 模型。

## 与 DeepSeek Harness / dsh 的差异

dsh 的架构文档非常明确：底层是 Cordis，产品每一部分都作为 plugin 挂到共享 context 上；profile、bundle、patch 决定运行时组合。它的 `packages/*` 覆盖 session、agent-loop、tools、llm、mcp、workflow、subagent、computer-use、browser-use、desktop/web/cli 等，整体更像可插拔 harness 平台。

Morpheus 与它的主要区别：

- dsh 的第一原则是 plugin-first：没有特权内核，runtime 由 Cordis plugin tree 组合。
- Morpheus 的第一原则是 thread/runtime facts：ThreadService、AppServer、Launcher、typed history、agent path 和 installed state 是核心边界。
- dsh 适合快速替换模型适配器、工具、profile、web/desktop/headless 组合；Morpheus 更适合一个长期运行的本地桌面 agent workspace，并把安装态更新、权限、replay 和 self-debug 做成产品纪律。
- dsh 的 desktop 是 profile/bundle 体系里的一个 app 形态；Morpheus 的 desktop 是当前产品主入口，Runtime Capsule 更新和 `/self` 源码项目围绕它设计。

## 开发布局

Morpheus 主 checkout：

```text
~/.morpheus/source_workspace
```

固定普通开发 checkout：

```text
~/.morpheus/source_workspace-dev
~/.morpheus/source_workspace-dev-2
~/.morpheus/source_workspace-dev-3
```

对照项目源码：

```text
~/.morpheus/reference-sources/openai-codex
~/.morpheus/reference-sources/claude-code-best-claude-code
~/.morpheus/reference-sources/deepseek-ai-deepseek-harness
```

## 构建和验证

普通 owner 开发阶段通常在自己的 dev checkout 跑 focused tests 和 debug build。完整 Runtime Capsule 构建、桌面重启、安装态 self-debug 验收由 PM 在 canonical main merge 后执行。

常见验证命令：

```shell
pnpm --filter @my-codex/root-worker-prototype test -- src/lib/conversation.test.ts
cargo build --manifest-path codex-rs/Cargo.toml -p app-server --bin app-server
pnpm package:root-worker-prototype:mac
```

## 上游文档

因为 Morpheus fork 自 Codex，底层 CLI/runtime 的很多说明仍可参考 upstream Codex 文档：

- [Codex documentation](https://developers.openai.com/codex)
- [Installing & building](./docs/install.md)
- [Contributing](./docs/contributing.md)

本仓库使用 [Apache-2.0 License](LICENSE)。

# Root Worker Project Agent Sidebar 设计文档

## 目标

将 root-worker 左侧栏从单一 agent tree 改造成以 project 为根节点的工作区导航。

新的用户模型是：

- 左侧栏根节点是 project，可以有多个 project 根节点。
- 一个 project 由一个 PM 管理，而不是由用户手动管理多个 project thread。
- 用户进入 project 后，实际是在和该 project 的 PM 工作。
- 现有 subagent tree 仍保留，作为 project PM 下派 owner / reviewer / worker 后形成的 agent 层级。
- 不绑定 project 的普通对话仍保留在独立 `Chat` 分组里。

这份文档是第一阶段实现契约。后续 owner brief、测试和代码实现都应以它为依据。

## 核心设计变化

旧模型：

```text
Agent Tree
  root thread
    subagent
    subagent
```

上一版文档中的中间模型：

```text
Project
  conversation root
    subagent tree
  conversation root
    subagent tree
```

现在采用的新模型：

```text
Project: my-codex
  PM
    owner_dev
    owner_dev_2
    reviewer

Project: codex-rs
  PM
    owner_dev

Chat
  Chat conversation
  Chat conversation
```

关键区别：

- Project 不是 conversation 的分组标签，而是用户感知上的根节点。
- Project 下不暴露多个 root conversation 让用户管理。
- 每个 project 有一个 PM 管理入口。
- Subagent tree 仍然存在，但它挂在 project PM 下。
- 底层仍可以用现有 thread 数据结构实现；只是 UI 和交互不再把 `root thread` 作为用户概念。

## 产品模型

### Project

Project 是一个 workspace / repository 级工作单元。

第一阶段可以从 PM thread 的 `cwd` 派生 project 身份：

- Project id：标准化后的 project `cwd`。
- Project label：从 `cwd` 派生的紧凑展示名，通常是路径最后一段。
- Project subtitle：裁剪后的完整路径。
- Project PM：该 project 的唯一 PM 管理 thread。
- Project status：聚合 PM 和所有 subagents 的状态。

用户不需要理解“root thread”。实现中如果仍使用 parentless thread 表示 PM thread，也应在 UI / 文案中称为 `PM` 或 `Project PM`。

第一阶段不要新增 backend project registry。UI 可以先用现有 thread metadata / `cwd` 派生 project 和 PM 映射；等 runtime 未来暴露一等 project metadata 后再迁移。

### Project PM

Project PM 是一个 project 的管理入口。

它负责：

- 接收用户对该 project 的目标、问题和 followup。
- 创建和协调 owner / reviewer / worker subagents。
- 汇总 project 当前状态、计划、待办和风险。
- 作为 project 下 subagent tree 的父节点。

UI 中 Project PM 的展示规则：

- Project node 展开后，第一行可以是 `PM`，也可以把 PM 状态直接合并到 project header。
- 如果 PM row 单独展示，它不应叫 `root`。
- 选择 project header 或 PM row 时，中央 conversation 显示 PM thread。
- Project header 应能表达 PM 或 subagent 的 active / waiting / failed 等聚合状态。

### Subagent Tree

现有 subagent tree 继续保留。

在 project 下：

- PM 是 subagent tree 的根。
- Direct subagents 是 PM 派发的 owner / reviewer / worker。
- Nested subagents 继续按现有 parent-child 关系缩进展示。
- 现有 subagent node 的 expand / collapse 状态继续有效。
- 现有非 PM agent 的 context menu 行为继续有效。

这次 redesign 改的是 project / PM 的用户模型，不改变 subagent tree 的核心语义。

### Chat 模式

Chat 模式是无 project 的普通对话模式。

Chat-mode conversations 应：

- 展示在独立的 `Chat` 分组下。
- 不绑定 repo / branch / worktree。
- 不假装拥有 cwd browser、repo diff 或 project workflows。
- 可以有自己的普通 conversation 列表，但不进入 project PM 管理模型。

`Chat` 是一个明确分组，不是未知 project 或坏数据的兜底垃圾桶。

## 左侧栏信息架构

左侧栏有三层：

1. 顶部操作区，例如 `New Project` / `New Chat` / search。
2. Project 根节点列表。
3. Project 内 PM + subagent tree。

推荐结构：

```text
Sidebar
  Header
    New Project / Open Project
    New Chat
    Search / filter

  Project: my-codex
    PM
      owner_dev
      owner_dev_2
      owner_dev_3
      reviewer

  Project: codex-rs
    PM
      owner_dev

  Chat
    General Q&A
    API question
```

Project header 是可折叠根节点，不是 conversation 分组标题。

Project header 应展示：

- Project name。
- 裁剪后的 cwd / path。
- PM 状态或 project 聚合状态。
- 当前 active / waiting / failed subagents 的轻量计数。
- 折叠/展开 affordance。
- 可选 project action menu。

PM row 应展示：

- `PM` 或 `Project PM`。
- PM thread 的短 preview / updated time。
- PM 状态指示。
- Descendant subagents 数量。
- 当 PM 或任意 subagent 被选中时，project header 要能表达 contains-selected 状态。

Subagent row 继续保留当前 tree 行为：

- Agent label / path。
- Role 或 subtitle。
- Status dot。
- Child count。
- Expand / collapse affordance。
- 可删除的非 PM agent context menu。

Chat group 应展示：

- `Chat` header。
- 普通 chat conversations 列表。
- Chat conversation 不展示 project cwd / worktree / diff affordance。

## 用户交互流程

### 1. 首次打开，已有 projects

当用户打开 root-worker，且已有 project PM threads：

1. 左侧栏展示 project 根节点列表。
2. 每个 project 是可折叠 section。
3. 默认展开最相关的 project：
   - 优先展开包含当前 selected / restored thread 的 project。
   - 否则展开最近更新的 project。
4. 展开的 project 下显示 PM 和现有 subagent tree。
5. 用户可以选择 project header / PM row 进入该 project 的 PM conversation。
6. 用户也可以展开 subagent tree，选择 owner / reviewer / worker。

用户预期结果：

- 用户看到的是“我有哪些 project”，不是“我有哪些 root thread”。
- 一个 project 只需要进入它的 PM；用户不需要手动维护多个 project thread。
- 现有 subagent 仍可按 tree 找到。

### 2. 首次打开，没有 projects

当没有 project PM threads：

1. 左侧栏展示空态，主操作是 `New Project` / `Open Project`。
2. 次操作是 `New Chat`。
3. 不应为了填充 tree 自动创建一个用户不可理解的 root thread。

用户预期结果：

- 第一个决策是“打开/创建哪个 project”，或者“开一个普通 chat”。

实现备注：

- 当前 `loadBootstrap()` 可能会通过 `ensureInitialRootThread(payload.workspace)` 自动创建初始 root thread。后续实现需要重新评估该行为。
- 如果兼容性要求必须创建 thread，应把它视为 current workspace 的 PM thread，并在 UI 中展示为 project PM，而不是展示为 root thread。

### 3. 打开或创建 project

用户意图：“开始管理一个 project。”

流程：

1. 用户点击 `New Project` 或 `Open Project`。
2. 打开紧凑 menu / popover。
3. Menu 列出：
   - 当前 workspace project，如果存在。
   - 已知 projects。
   - 可选的 path 输入 / browse 入口，后续迭代实现。
4. 用户选择一个 project。
5. 如果该 project 已有 PM thread：
   - 选中该 project。
   - 展开 project。
   - 中央 conversation 显示 PM thread。
6. 如果该 project 没有 PM thread：
   - 创建一个 PM thread，例如底层调用 `createThread({ cwd: project.cwd, name: "PM" })`。
   - 新 project node 插入左侧栏。
   - 新 project 展开。
   - 中央 conversation 显示 PM thread composer。

可见反馈：

- 创建中时，在对应 project 位置显示 pending 状态。
- 创建失败时，在 sidebar 附近展示可恢复错误。

硬性要求：

- 不要让用户为同一个 project 创建多个并列 root conversations。
- 如果同一个 project 已存在 PM thread，新操作应进入已有 PM，而不是新建重复 PM。
- UI 文案中不要出现 `root thread` 作为用户动作对象。

### 4. 新建 chat conversation

用户意图：“不绑定 project，直接进行普通对话。”

流程：

1. 用户点击 `New Chat`，或在 `New Project` menu 中选择 `Chat without project`。
2. App 在后端支持的前提下创建无 project `cwd` 的 chat conversation。
3. `Chat` group 展开。
4. 新 chat conversation 被选中。
5. 中央 composer 进入 chat mode。

Chat-mode UI 行为：

- Sidebar group label 是 `Chat`。
- Conversation header 不显示 repo / worktree affordances。
- Project-only controls 隐藏，或以清晰文案 disabled。
- File browser、repo diff、project workflow 等能力不能假装可用。

后端不确定项：

- 如果 `createThread` 不能创建无 `cwd` thread，则 chat option 应 disabled 或标记为 unavailable。
- 实现不能偷偷传 `workspace` 作为 `cwd`，然后把它称为 chat。

### 5. 切换 project

用户意图：“从一个 project 切换到另一个 project。”

流程：

1. 用户扫描 project headers。
2. 用户点击一个 project header。
3. 如果 project 已折叠，则展开它。
4. 中央 conversation 显示该 project 的 PM thread。
5. 其他 project 可保持原折叠状态；除非最终 UI 明确选择 accordion 单开模式，否则不要强制只展开一个。

用户预期结果：

- 切换 project 是一次导航行为。
- 它不会创建新 PM。
- 它不会重置其他 thread 的 composer draft。
- 用户不需要在同一个 project 下挑选多个 root conversation。

### 6. 和 Project PM 交互

用户意图：“给这个 project 的 PM 下达目标或 followup。”

流程：

1. 用户点击 project header 或 PM row。
2. Project / PM 进入 selected 状态。
3. 中央 conversation 显示 PM thread。
4. 用户在 composer 中输入目标、问题或 followup。
5. PM 可以继续派发 subagents；这些 subagents 出现在该 project 下的 subagent tree 中。

用户预期结果：

- Project 是工作入口，PM 是执行协调者。
- 用户不需要手动创建多个 project thread 来表达不同任务。

### 7. 选择 subagent

用户意图：“查看或 follow up 某个 owner / reviewer / worker。”

流程：

1. 用户展开 project。
2. 如有必要，展开 PM 下的 subagent tree。
3. 用户点击 subagent row。
4. Subagent row 进入 selected 状态。
5. 中央 conversation 切换到该 subagent thread。
6. 现有 followup / composer 行为保持不变。
7. 现有非 PM agent context menu 继续可用。

用户预期结果：

- Project grouping 不隐藏、不弱化 subagent tree。
- 用户仍能按 canonical tree structure 导航 fixed owners 和 nested agents。

### 8. 折叠和重新打开

用户意图：“减少 sidebar 噪声，同时不丢失当前位置。”

流程：

1. 用户折叠一个 project。
2. 该 project 内 PM 和 subagents 从 sidebar 中隐藏。
3. Project header 仍显示聚合状态和 active / waiting / failed 计数。
4. 如果 collapsed project 内有 active / failed descendant 更新，project header 状态随之变化。
5. 重新打开 project 时，恢复之前的 subagent tree collapse state。

选中态边界：

- 如果 selected thread 位于用户刚折叠的 project 内，中央 thread 仍保持选中。
- Collapsed project header 应显示 selected / contains-selected 状态，让用户知道当前 selected thread 在这个 project 里。

### 9. 删除或归档 subagent tree

用户意图：“移除某个 agent 或 subagent subtree。”

流程：

1. 用户在非 PM subagent row 上打开 context menu。
2. 出现现有 delete / archive action。
3. Action 按现有语义归档该 subagent subtree。
4. Project 保留。
5. PM 保留。
6. Project header 和 PM row 的 subagent count / status 更新。

非目标：

- 第一阶段不新增 project-level delete / archive。
- 第一阶段不允许通过 subagent menu 删除 PM。
- 如果未来需要关闭 project，应单独设计 project archive flow。

### 10. 搜索或过滤

第一阶段 search 可以保持最小，但交互位置和预期要清楚：

- Search 应匹配 project labels、PM label、subagent labels、chat conversation labels 和 canonical paths。
- 命中结果所在的 project 应展开，或以某种方式展示 matched descendants。
- 清空 search 后恢复搜索前的 collapse state。

如果第一轮代码不实现 search，也应在 header 中保留稳定位置，避免后续再改信息架构。

## 创建入口

建议拆成两个清晰入口：

- `New Project` / `Open Project`：进入 project PM 模式。
- `New Chat`：进入无 project 的普通 chat 模式。

如果 UI 空间需要合并，也可以使用一个 `New` 按钮，菜单内分组：

```text
Project
  Current Project: my-codex
  Existing Project: codex-rs
  Open Project...

Chat
  Chat without project
```

Project 创建行为：

- Project 已存在 PM：选择并打开该 PM。
- Project 不存在 PM：创建 PM thread，底层可调用 `createThread({ cwd: project.cwd, name: "PM" })`。
- 新建或打开后，project 展开且 PM 被选中。

Chat 创建行为：

- 如果 bridge 支持 no-project：创建无 `cwd` conversation。
- 如果 bridge 暂不支持 no-project：禁用 chat 创建，显示明确原因。
- 不要用 workspace cwd 伪造 chat。

## 选择行为

选择任何 row 仍然只选择一个 thread id。

预期行为：

- 点击 project header：
  - 如果只是导航点击，则选择 PM thread 并展开 project。
  - 如果点击 chevron，则只切换折叠状态。
- 点击 PM row：选择 PM thread。
- 点击 subagent：选择该 subagent thread。
- 点击 chat conversation：选择该 chat thread。
- 如果 selected thread 在 collapsed project 内，重新打开应用时应保持该 project 展开，或自动展开它。
- 如果 selected subagent 位于 collapsed subagent tree 内，应自动展开到足以让 selected item 可见。

中央 conversation view 仍展示 selected thread。不同点是：用户选择 project 时，selected thread 是该 project 的 PM。

## 分组规则

Project membership 由 PM thread 决定。

第一阶段实现可以按现有 thread 结构映射：

1. 找到 parentless、带 project `cwd` 的 thread，将它视为 project PM。
2. 用 PM thread 的 `cwd` 派生 project key。
3. 同一个 project key 下如果出现多个 parentless threads：
   - UI 不应把它们暴露成多个 root conversations。
   - 应选择一个 canonical PM，优先使用最近 active / 最近 updated 的 thread。
   - 其他重复 parentless threads 作为数据冲突风险处理，可隐藏到 debug / overflow，或后续迁移合并；不要成为主交互模型。
4. PM 的 descendants 构成该 project 的 subagent tree。

对 subagents：

- 不要按 subagent 自己的 `cwd` 独立分 project。
- Subagent 继承 PM 所属 project。
- Subagent 可以在其他详情区域展示自己的 cwd，但 sidebar project grouping 不因此拆分。

对 chat：

- 无 project `cwd` 的 parentless chat threads 展示在 `Chat` group 下。
- Chat conversation 可以是多个，因为 chat 不由 project PM 管理。

排序：

- Projects 按 PM 或 descendants 最近更新时间倒序排列。
- `Chat` 默认在 projects 后面；如果包含 selected chat，可以保持展开/突出，但不要产生不可预测的跳动。
- Project 内 subagent children 保留现有 created-at 顺序，除非当前 tree 已定义更明确顺序。
- Chat conversations 按最近更新时间倒序排列。

## 状态聚合

Project status 应从 PM 和 descendants 聚合。

优先级：

1. Failed / system error。
2. Active / running。
3. Waiting on user / subagent / subscription / tool。
4. Stale / restored / not loaded。
5. Complete / idle。

Project header 和 PM row 都应能暴露关键状态：

- Project header 展示聚合状态和轻量计数。
- PM row 展示 PM 自身状态。
- Subagent row 继续使用当前 `treeThreadStatusClass(...)` 语义。

这样即使 active subagent 藏在折叠 tree 深处，project header 也能让用户看到有事发生。

## 折叠状态

折叠状态按层拆开。

- Project collapse keys 使用稳定 project keys，不使用 thread ids。
- Chat group collapse key 使用稳定 `chat`。
- PM / subagent tree collapse 可继续使用 thread ids。
- 不要不加 namespace 地复用 `collapsedPaths` 同时表示 project 和 tree node；这会造成 key 冲突和持久化语义混乱。

建议状态：

```ts
type SidebarCollapseState = {
  collapsedProjects: string[];
  collapsedTreeNodes: string[];
  collapsedChat: boolean;
};
```

如果实现阶段临时继续使用现有 `collapsedPaths`，project keys 必须加前缀，例如 `project:/Users/...`，chat 使用 `group:chat`。

## 现有代码映射

当前相关入口：

- `apps/root-worker-prototype/src/App.tsx`
  - 持有 `threads`、`workspace`、`selectedThreadId`、`newRootName` 和 collapse state。
  - 当前会派生 `selectedTreeRootId`，过滤 `sessionThreads`，然后把单个 `agentTree` 传给 `SidebarPanel`。
  - `createRootThread(...)` 当前默认 `cwd = workspace`。
  - 后续应改成 project PM 选择 / 创建入口；UI 文案不要继续叫 root。

- `apps/root-worker-prototype/src/components/Panels.tsx`
  - `SidebarPanel` 当前渲染标题为 `Agent Tree` 的 `AgentTreeNode` 列表。
  - 后续应改为渲染 project roots、PM row、subagent tree 和 Chat group。

- `apps/root-worker-prototype/src/components/AgentTree.tsx`
  - `AgentTreeNode` 应继续负责渲染 PM/subagent tree node 及其 descendants。
  - 可以复用它渲染 PM 下的 subagent tree，但显示文案上不要暴露 `root`。

- `apps/root-worker-prototype/src/lib/thread.ts`
  - `buildAgentTree(...)` 构建 parent / child tree。
  - 可新增纯 helper，例如 `buildProjectAgentSidebar(...)`，负责把 PM threads 派生成 project roots，并嵌入 PM/subagent `TreeNode`。

- `apps/root-worker-prototype/src/types.ts`
  - 如有需要，新增 typed sidebar group structures。

## 建议类型

```ts
export type SidebarProjectNode = {
  id: string;
  label: string;
  subtitle: string | null;
  cwd: string;
  statusClass: TreeThreadStatusClass;
  updatedAt: number;
  pmTree: TreeNode;
  descendantCount: number;
  activeCount: number;
  waitingCount: number;
  failedCount: number;
  duplicatePmThreadIds?: string[];
};

export type SidebarChatGroup = {
  id: "chat";
  statusClass: TreeThreadStatusClass;
  updatedAt: number;
  conversations: TreeNode[];
};
```

具体类型名可调整，但关键区分必须保留：

- `SidebarProjectNode` 是 project level。
- `pmTree` 是 PM + subagent level。
- Chat conversations 是无 project 的普通 conversation，不参与 project PM 模型。

## 视觉方向

使用简洁、接近 Codex 的 sidebar：

- 白色或近白底色。
- 细分隔线。
- 紧凑 rows。
- Project 是根节点 section。
- Project header 清晰但克制。
- PM row 和 subagent rows 视觉上属于同一个 tree。
- 不使用大卡片。
- 不引入 bottom dock 或 bottom panel。

左侧栏应像 project navigator，不像 dashboard。

Project header 可以比普通 row 稍强，但不能变成大卡片。用户应能快速扫描多个 projects，并进入每个 project 的 PM。

## 非目标

第一阶段不实现：

- Backend project registry。
- Project rename / delete。
- Project archive / close。
- Sidebar 内 git branch / worktree 管理。
- Cross-project search。
- 拖拽 subagents 或 chats 到 project。
- 持久化自定义排序。
- 多选或批量归档。
- 重写中央 conversation 或右侧 panel。
- 展平或移除当前 subagent tree。
- 让一个 project 在 UI 上暴露多个 root conversations。

## 验收标准

行为：

- 左侧栏根节点是 projects，可以有多个 project 根节点。
- 每个 project 由一个 PM 管理入口表示。
- UI 不再把 root thread / root conversation 作为用户概念展示。
- 选择 project header 或 PM row 后，中央 conversation 显示该 project 的 PM。
- PM 下的现有 subagent tree 仍可见、可选择。
- Chat-mode conversations 聚合到独立 `Chat` group。
- `New Project` / `Open Project` 进入 project PM；`New Chat` 进入 chat mode。
- Project / chat collapse 不破坏 subagent tree collapse。

数据正确性：

- Subagents 继承其 PM 所属 project。
- Project count 统计 projects，不统计多个 root conversations。
- 同 project 多 parentless thread 不应成为多个主 UI 根节点。
- 状态聚合能在 project 层暴露 active / failed / waiting descendants。
- Selected thread 所在 project 和 tree 能被展示或自动展开。

实现：

- Grouping 逻辑要有 `src/lib/thread.test.ts` 或新 focused test file 的纯函数测试。
- Sidebar rendering 要覆盖：
  - 多个 project roots。
  - 一个 project 下 PM + nested subagent tree。
  - `Chat` group 下多个普通 chat conversations。
  - Project collapse 与 tree collapse 相互独立。
  - New Project / New Chat 创建入口。
  - 同 project 重复 PM/root 数据不会显示成多个主 project conversations。
- 产品代码不能用 workspace cwd 伪造 chat mode。

验证：

- 运行 root-worker 相关 grouping 和 sidebar rendering 定向测试。
- 如果改到 production TypeScript / React code，运行 `rtk pnpm --dir apps/root-worker-prototype build`。

## 开放问题

1. Project PM 的底层 thread 如何稳定识别？只用 parentless + cwd，还是需要后端补 metadata？
2. 同一个 project 已有多个 parentless threads 时，第一阶段是选最近 updated 作为 canonical PM，还是需要 migration / picker？
3. Chat mode 应发送 `cwd: ""`、omit `cwd`，还是等待后端 API 变更？
4. 第一版 `Open Project` 是否只列出已有 projects，还是即使没有 PM thread 也列出当前 bootstrap workspace？
5. Project collapse state 第一阶段是 session-local，还是沿用现有 local storage 模式持久化？
6. Project grouping 使用 raw `cwd`，还是在后端可用时使用 canonicalized path？

建议第一阶段答案：

1. 先用 parentless + normalized `cwd` 派生 PM，并把 UI 文案改成 PM / Project。
2. 第一阶段选择最近 active / updated 的 thread 作为 canonical PM，并记录重复数据为实现风险，不在主 UI 暴露多个 root conversations。
3. 先验证 bridge 行为；不要伪造 chat mode。
4. 包含当前 bootstrap workspace 作为 project option。
5. 除非附近已有合适 local storage 模式，否则 collapse state 先保持 session-local。
6. 先用 normalized raw `cwd`，等后端提供 canonical project metadata 后再迁移。

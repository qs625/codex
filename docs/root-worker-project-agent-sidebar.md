# Root Worker Project Agent Sidebar 设计文档

## 目标

将 root-worker 左侧栏从单一 agent tree 改造成以 project chat 为根的工作区导航。

新的用户模型是：

- 左侧栏根节点是 projects，可以有多个 project。
- 每个 project 的 chat thread 本身就是该 project 的 tree root。
- 用户选择 project 后，中央 conversation 进入这个 project chat。
- Project 展开后只显示该 project chat 派生出的 subagents，不额外显示 PM/root 中间节点。
- 不绑定 project 的普通对话保留在独立 `Chat` 分组里。
- 顶部只有一个新建/打开入口；用户在入口里选择 current project，或选择不绑定 project 的 chat。

这份文档是第一阶段实现契约。后续 owner brief、测试和代码实现都应以它为依据。

## 核心设计变化

旧模型：

```text
Agent Tree
  root thread
    subagent
    subagent
```

当前模型：

```text
Project: my-codex
  owner_dev
    reviewer
  worker

Project: codex-rs
  owner_dev

Chat
  General Q&A
    helper
  API question
```

关键区别：

- Project header 代表 project chat root thread。
- Project 下不再额外渲染 `PM`、`Project PM`、`root` row。
- Subagent tree 仍然存在，但它直接挂在 project chat root 下。
- 底层仍可以使用 parentless thread + `cwd` 表示 project chat；UI 不把 `root thread` 作为用户概念。

## 产品模型

### Project

Project 是一个 workspace / repository 级工作单元。

第一阶段从 parentless thread 的 `cwd` 派生 project 身份：

- Project id：标准化后的 project `cwd`。
- Project label：从 `cwd` 派生的紧凑展示名，通常是路径最后一段。
- Project subtitle：裁剪后的完整路径。
- Project chat root：该 project 的 canonical parentless thread。
- Project status：聚合 project chat root 和所有 descendants 的状态。

同一个 project 出现多个 parentless threads 时，第一阶段只选择一个 canonical project chat root 展示。其余 duplicate roots 不作为多个 project rows 暴露。

### Project Chat Root

Project chat root 是用户进入 project 后看到的 conversation。

UI 展示规则：

- Project header 点击后选中 project chat root thread。
- Project header 展示 project 名称、路径、聚合状态和 active / waiting / failed / agents 计数。
- Project header 吸收 root thread 的选择行为和状态；展开后不再显示一行 root/PM。
- Project header 处于 selected root 或 selected descendant 时，应表达 contains-selected 状态。

### Subagent Tree

现有 subagent tree 继续保留。

在 project 下：

- Direct subagents 是 project chat root 派发的 owner / reviewer / worker。
- Nested subagents 继续按现有 parent-child 关系缩进展示。
- 现有 subagent node 的 expand / collapse 状态继续有效。
- 现有非 root agent 的 context menu 行为继续有效。

这次 redesign 改的是 project/root 的用户模型，不改变 subagent tree 的核心语义。

### Chat 模式

Chat 模式是无 project 的普通对话模式。

Chat-mode conversations 应：

- 展示在独立的 `Chat` 分组下。
- 不绑定 repo / branch / worktree。
- 不假装拥有 cwd browser、repo diff 或 project workflows。
- 可以有自己的 descendants，并按 tree 展示。

当前后端如果仍会把 missing cwd default 到 workspace，UI 不能用 workspace cwd 伪造 no-project chat；应明确给出 disabled/gated 反馈，直到后端支持真正的 no-project thread。

## 左侧栏信息架构

推荐结构：

```text
Sidebar
  Header
    New
      Current workspace
      Chat without project

  Project: my-codex
    owner_dev
      reviewer
    worker

  Project: codex-rs
    owner_dev

  Chat
    General Q&A
      helper
    API question
```

顶部只有一个主要新建/打开入口：

- `Current workspace`：已有当前 workspace project chat 时选中它；不存在时用当前 workspace `cwd` 创建。
- `Chat without project`：后端支持时创建 no-project chat；后端不支持时必须给出明确反馈，不可静默无效。

Project header 是可折叠根节点，不是 conversation 分组标题。

Project header 应展示：

- Project name。
- 裁剪后的 cwd / path。
- Project 聚合状态。
- 当前 active / waiting / failed subagents 的轻量计数。
- 折叠/展开 affordance。
- 可选 project action menu。

Subagent row 继续保留当前 tree 行为：

- agent nickname / role。
- status。
- preview / last activity。
- collapse state。
- context menu。

## 交互流程

### 1. 启动与已有 projects

当用户打开 root-worker，且已有 project chat roots：

1. Sidebar 使用 parentless threads 按 `cwd` 派生 projects。
2. 每个 project 只展示一个 canonical project root。
3. Project header 点击后选中该 root thread。
4. Project 展开后显示 root 的 direct subagents 和 nested descendants。
5. `Chat` group 展示 no-cwd parentless conversations。

### 2. 空态

当没有 projects：

1. 左侧栏展示空态。
2. 主入口仍是顶部单个 `New` 按钮。
3. 空态可提供 current workspace 的 `Open` 快捷操作。
4. 如果没有 workspace cwd，应显示明确错误，不创建 project chat。

### 3. 新建/打开 current workspace project

1. 用户点击 `New`。
2. 用户选择 `Current workspace`。
3. 如果当前 workspace 已有 project chat root，直接选中并展开对应 project。
4. 如果不存在，调用 create thread，并传入当前 workspace `cwd`。
5. 新 thread 进入 project list，作为该 project 的 root。

### 4. 新建普通 chat

1. 用户点击 `New`。
2. 用户选择 `Chat without project`。
3. 若后端支持 no-project thread，则创建并放入 `Chat` group。
4. 若后端不支持，则展示明确错误或 disabled 状态。

### 5. 选择与展开

- 选择 project header：选中 project chat root。
- 选择 subagent row：选中该 subagent thread。
- 选择 Chat conversation：选中该 chat root。
- 选择 Chat descendant：选中该 descendant thread。
- 如果 selected thread 位于 collapsed project/chat/tree 内，UI 应自动展开可见路径。

## 数据与类型建议

可新增纯 helper，例如 `buildProjectAgentSidebar(...)`：

- 输入：`Thread[]`。
- 输出：project list + chat group。
- 不访问 React state。
- 不调用 backend。

建议类型：

```ts
export type SidebarProjectNode = {
  id: string;
  label: string;
  subtitle: string | null;
  cwd: string;
  statusClass: TreeThreadStatusClass;
  updatedAt: number;
  tree: TreeNode;
  descendantCount: number;
  activeCount: number;
  waitingCount: number;
  failedCount: number;
  duplicateRootThreadIds?: string[];
};

export type SidebarChatGroup = {
  id: "chat";
  statusClass: TreeThreadStatusClass;
  updatedAt: number;
  conversations: TreeNode[];
};
```

关键区分：

- `SidebarProjectNode` 是 project level。
- `tree` 是 project chat root + subagent descendants。
- UI 可以用 project header 吸收 `tree` 的 root node，不必把 root node 额外渲染成一行。
- Chat conversations 是无 project 的普通 conversation，不参与 project grouping。

## 验收标准

行为：

- 左侧栏根节点是 projects，可以有多个 project 根节点。
- 每个 project 的 chat/root thread 本身就是 tree root。
- Project 下不额外显示 `PM` / `Project PM` / `root` row。
- 选择 project header 后，中央 conversation 显示该 project chat root。
- Project 展开后仍能看到并选择 subagents。
- Chat-mode conversations 聚合到独立 `Chat` group。
- 左侧顶部只有一个主要新建/打开入口。
- 点击该入口有可见反应，并提供 project / no-project chat 选择或明确 gated feedback。
- Project / chat collapse 不破坏 subagent tree collapse。

数据正确性：

- Subagents 继承其 project root 所属 project。
- Project count 统计 projects，不统计多个 duplicate roots。
- 同 project 多 parentless thread 不应成为多个主 UI 根节点。
- 状态聚合能在 project 层暴露 active / failed / waiting descendants。
- Selected thread 所在 project 和 tree 能被展示或自动展开。

实现：

- Grouping 逻辑要有纯函数测试。
- Sidebar rendering 要覆盖：
  - 多个 project roots。
  - project 展开显示 nested subagent tree，且不显示额外 root/PM row。
  - `Chat` group 下多个普通 chat conversations。
  - Project collapse 与 tree collapse 相互独立。
  - 单一 `New` 入口。
  - 同 project duplicate parentless roots 不显示成多个 projects。
- 产品代码不能用 workspace cwd 伪造 chat mode。

验证：

- 运行 root-worker 相关 grouping 和 sidebar rendering 定向测试。
- 如果改到 production TypeScript / React code，运行 `rtk pnpm --dir apps/root-worker-prototype build`。

## 开放问题

1. Project chat root 的底层 thread 如何稳定识别？第一阶段使用 parentless + cwd，未来可能需要后端 metadata。
2. 同一个 project 已有多个 parentless threads 时，第一阶段选择 canonical root，未来是否需要 migration / picker？
3. Chat mode 应发送 `cwd: ""`、omit `cwd`，还是等待后端 API 变更？
4. 第一版 `New -> Current workspace` 是否只支持 bootstrap workspace，还是支持完整 project picker？
5. Project collapse state 第一阶段是 session-local，还是沿用现有 local storage 模式持久化？

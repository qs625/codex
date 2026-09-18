---
name: ui-design
description: "my-codex 的 UI 设计流程。适用于设计或实现新 UI、root-worker 页面/面板/dashboard/视觉组件/前端交互；尤其适用于需要先用 image_gen 生成统一设计锚点，再让后续组件延展同一设计语言的任务。"
---

# UI 设计流程

做 UI 任务时使用本 skill。目标是避免模型从需求直接跳到写代码，也避免把 image_gen 当成“凭空想 UI”的入口。稳定流程是：先把产品语义和交互 contract 说清楚，再把它压成非常具体的 image_gen brief，最后从图里提炼可执行 design brief，让后续组件持续延展同一套视觉系统。

## 核心规则

有意义的 UI 改动，不要从产品请求直接进入代码实现，也不要在产品语义还薄的时候直接调用 image_gen。

任何非平凡 UI 任务，都要先产出一份语义 contract，再产出或复用一个设计锚点：

1. 先整理产品语义：用户任务、入口、运行中状态、确认/取消/接管、错误恢复、审计证据、完成证明、与现有界面的关系、禁止误导用户的表达。
2. 如果已有 design brief / pattern library，优先复用。
3. 如果已有截图或已实现界面清楚定义了视觉语言，优先复用。
4. 如果设计语言、布局密度、信息层级或视觉处理还不清楚，基于第 1 步语义 contract 用一次 `image_gen` 生成单张完整 composite mockup。

不要为多个组件分别独立生成图片。这样会导致风格漂移。

## 什么时候使用 image_gen

以下情况使用 `image_gen`：

- 新页面、主要面板、dashboard、inspector、timeline、graph、command center 或复杂工作流。
- 新组件会改变主要工作流，或引入新的视觉模式。
- 布局密度、信息层级、空态/加载/错误态、响应式行为还不明确。
- 如果直接实现，很容易产出泛化、粗糙或与现有产品不匹配的 UI。

以下情况跳过 `image_gen`：

- 组件很小，已有模式足够覆盖。
- 项目现有 UI 已经清楚定义了同类模式。
- 只是文案、间距 polish，或简单状态修复。

## 首次设计流程

### 1. 定义产品语义 contract

只记录实现真正需要的信息：

- 目标用户和要完成的任务。
- 核心工作流和成功标准。
- 需要展示的数据、支持的判断、主要操作。
- 必须覆盖的状态：空态、加载、active、选中、完成、失败、禁用；my-codex 场景还要考虑 stale/restored。
- 入口/触发方式：用户从哪里发起、是否需要确认、如何取消/接管。
- 运行中反馈：系统如何表达正在做什么、目标是谁、下一步是什么。
- 审计证据：哪些事实必须可见，哪些只能摘要展示，哪些不能长期保存。
- 错误与权限：权限不足、目标不匹配、风险动作、恢复/重试如何表达。
- 与现有 surface 的关系：conversation、RightPanel、Browser、Terminal、Live Commands、桌面 overlay 等各自承担什么，不承担什么。
- 禁止项：哪些 UI 形态或文案会误导用户，哪些实现路径会偷换产品语义。

这一节要先写成人能判断的 brief，再进入 image_gen。image_gen 的 prompt 应该是语义 contract 的压缩和视觉化，不应替代这个判断。

### 2. 生成单张整体 mockup

如果需要视觉方向，只调用一次 `image_gen`，生成完整页面或真实上下文中的完整区域。mockup 应同时包含主要布局和代表性状态。调用前必须已经有第 1 步语义 contract，并把关键语义明确写进 prompt。

适合生成：

- 完整应用页面。
- 完整右侧面板。
- 带真实数据区域的 dashboard。
- timeline + inspector。
- 带实际控件的 settings / management surface。

避免生成：

- 脱离页面上下文的孤立 button、card、badge、modal。
- 分别生成 header / sidebar / card / modal。
- 给操作型工具生成装饰性 hero 图。

推荐 prompt：

```text
Create a complete UI mockup for <screen/panel/workflow>.
Audience: <target user>.
Primary task: <task>.
Product semantics:
- <what the UI means and must not imply>
- <which real system/app the action affects>
- <what evidence/audit must be visible>
Workflow:
- Entry: <how user starts>
- Active state: <what feedback is shown>
- Control/override: <confirm/cancel/stop/takeover>
- Completion/error: <how result or failure is proven>
Show realistic data and these states: <states>.
Design constraints: operational product, high scanability, restrained visual style, compact but readable density, stable layout, no marketing hero, no decorative gradient/orb background.
Forbidden visuals or implications:
- <do not show misleading surface/relationship>
- <do not hide important state or safety evidence>
Output one cohesive design language for layout, spacing, typography, color, controls, status indicators, and interaction affordances.
```

### 3. 审图并提炼 design brief

生成 mockup 后，不要直接照图写代码。先审图：确认它是否尊重语义 contract，是否误导用户，是否把某个 surface 的职责画错，是否过度装饰或过度庞大。只有通过审图的部分才能进入实现 brief；不符合语义的细节要明确剔除或改写。

必须包括：

- 产品气质和信息密度。
- 布局结构和滚动边界。
- 字体层级。
- 色彩 token 和语义状态色。
- 间距、圆角、边框、阴影规则。
- button、toolbar、tab、input、badge、table/list、card/panel 的样式规则。
- 交互状态和响应式行为。
- 明确禁止项。
- 从 mockup 中保留的规则。
- 从 mockup 中拒绝的规则，以及拒绝原因。

这份 brief 才是实现依据。

### 4. 按 brief 实现

owner 实现时必须遵守 design brief 和项目现有模式。优先复用本地组件、CSS 习惯、图标体系和状态/数据流。

实现必须覆盖目标用户自然会遇到的状态，不能只做静态 mock。

### 5. 做视觉和行为验证

前端 UI 验证至少考虑：

- 相关 unit / component tests。
- 复杂 UI、响应式布局、canvas/视觉重组件要做截图或 Playwright 验证。
- 长文本、空态、加载、错误、窄屏。
- 如果 UI 依赖 persisted thread state，要检查 reload/restored path。

review 时先验设计，再看测试。测试通过不代表 UI 符合设计。

## 后续新增组件

不要为新增组件重新生成一套孤立风格。

先检查 design brief 或已有 pattern library 是否已经覆盖该组件：

- 已覆盖：直接按已有模式实现。
- 未覆盖：只允许用 `image_gen` 生成“同一设计系统下的扩展稿”。

扩展稿必须把新组件放在真实上下文中：右侧面板、timeline row、settings 页面、table、inspector 等。

扩展稿推荐 prompt：

```text
Extend this existing design system without changing its visual language.

Existing design brief:
<brief>

New component:
<component and purpose>

Placement/context:
<where it appears in the existing UI>

Required data and states:
<states>

Keep the same color system, typography hierarchy, spacing density, border radius, controls, icon-button style, status badge treatment, and panel/list/table language.
Do not redesign the product. Produce a contextual extension mockup, not an isolated component.
```

生成扩展稿后，要把新增规则回写到 design brief 或 component pattern notes：

- 新增组件模式。
- 复用的 token 和控件规则。
- 新增语义状态样式。
- 只属于该组件的局部特例，不能误升级为全局规则。

## PM / Owner 交接

UI 任务的 PM brief 应包含：

- 是否已有设计锚点。
- 如果需要 `image_gen`，明确要生成 composite mockup 还是 extension mockup。
- design brief，或要求 owner 在编码前先产出 design brief。
- 必须覆盖的状态和响应式检查。
- 需要遵循的现有项目 UI 文件。
- 禁止路径：孤立组件图、多次独立 image_gen、前端掩盖后端事实缺失、装饰性重设计、直接 code-first 实现。

owner 交付应包含：

- 使用的设计锚点：现有 UI、截图，或 `image_gen` mockup。
- 提炼或更新后的 design brief。
- 修改文件。
- 已做的视觉/状态验证。
- 为未来组件沉淀的新 pattern。

## Review 清单

接受 UI 工作前确认：

- 实现遵守 design brief，而不是只满足文字需求。
- 新组件融入现有产品语言。
- 长内容和窄屏下布局稳定。
- 主操作、次操作、危险操作视觉层级清楚。
- 状态明确，不靠含糊文案糊过去。
- UI 没有掩盖后端/runtime 事实缺失。
- 没有夹带无关视觉重设计。
- 如果用了多次 `image_gen`，后续生成必须明确继承第一次的设计系统。

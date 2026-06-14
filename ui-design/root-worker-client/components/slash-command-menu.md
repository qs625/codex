# SlashCommandMenu 组件规范

## 组件拆分

- `SlashCommandMenu`：连接 composer draft、候选数据、键盘事件和选择动作。
- `SlashMenuOverlay`：负责锚定 composer、视口避让、滚动高度。
- `SlashMenuGroup`：展示 `Commands`、`Skills` 以及分组级 loading/error。
- `SlashMenuItem`：候选行，支持 active、hover、disabled、selected/added。
- `SkillChip`：composer 内 skill token，复用现有 chip 视觉和 payload 行为，本次不重定义。

## Props / 数据

内置命令：
- `commandId: string`
- `token: string`
- `label: string`
- `description: string`
- `aliases?: string[]`

本次命令 registry：

| commandId | token | description | selection |
| --- | --- | --- | --- |
| `clear` | `/clear` | 归档当前会话并新建 root | 立即执行当前 `/clear` handler，不发送普通消息 |

Skill：
- 沿用当前 `name: string`、`path: string`。
- 可选增强：如果 Electron 透传 app-server metadata，可使用 `shortDescription` / `description` 辅助说明。

## 视觉规格

- 弹层半径沿用当前客户端 8px 内的规则，阴影保持轻量。
- 宽度跟随 composer 主输入区，最小 420px，窄屏贴合 composer 容器。
- 分组标题使用 12px uppercase/semibold，颜色弱于候选主文本。
- 候选行高 40-44px；主文本单行，说明单行截断。
- active 行使用浅蓝背景和边框/左侧线，不能只靠颜色表达。
- loading/error/empty 行不可选，文字比候选弱一级。

## 状态机

- `closed`：无 slash token 或用户关闭。
- `openIdle`：slash token 存在，query 为空。
- `filtering`：query 非空，列表按结构化字段过滤。
- `skillsLoading`：Skills 数据请求中。
- `skillsError`：Skills 请求失败。
- `empty`：没有任何可选候选。

状态优先级：`closed` > `skillsError` / `skillsLoading` 与候选列表并存 > `empty`。

## 交互规则

- 菜单打开时不 blur composer。
- active index 只落在可选候选上。
- query 改变后 active index 回到第一条可选候选。
- `Tab` 对候选做补全，不发送消息。
- `Enter` 对 Skill 走现有选择/chip/payload 行为，对本次范围内的无参数内置命令执行。
- `Escape` 关闭菜单，不删除 slash token。

## 可访问性

- composer 暴露 `aria-expanded`、`aria-controls`、`aria-activedescendant`。
- 候选行有稳定 id，active descendant 指向当前行。
- 分组标题不进入 tab order。
- loading/error 使用 polite live region。
- 鼠标和键盘选择走同一 handler，避免状态分叉。

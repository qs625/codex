# 设计 review

## 当前状态

已完成独立 `@ui-ue-reviewer` review，并按反馈修订。

## review 范围

- UX：左右对齐和连续合并是否提高长线程扫读效率。
- UI：桌面/移动宽度、间距、圆角、阴影和 segment 边界是否清晰。
- Accessibility：键盘、屏幕阅读器、颜色对比和阅读顺序。
- Engineering：是否遵守 typed `ThreadItem -> ConversationEntry -> ConversationCell` 路径，不新增 raw marker 解析。
- Content：状态文案是否简洁、可本地化。

## review 结论

初次结论：未通过。问题集中在工程边界命名、typed 状态断开规则、Electron baseline 后续验收和正文行宽。

修订结果：

- 将 `message:mixed-system` 改为 UI 派生 presentation state `message:agentGrouped`，并明确不得新增 `ConversationCell.kind`、protocol 类型或 raw 展示分支。
- 明确 error/cancelled/permission required 的断开规则必须按 typed source 决定：`agentMessage` 留在 message segment，tool/event/collab/hook/permission 轨迹保持对应 cell kind。
- 在 brief 中补充真实 Electron baseline/after 截图是实现验收必需项，当前 fallback 空白截图不可作为视觉验收依据。
- 将正文理想行宽从 72-88 characters 收敛到 64-78 characters，代码块作为例外。

复审判断：通过。修订已覆盖 reviewer 所列阻塞问题，不需要重新生成 prototype 图。

## 待确认项

- Electron 真实 baseline 未能获取；当前只有 Vite renderer fallback 空白截图，开发 PR 中必须补采真实 baseline/after。
- RTL 语言环境下是否镜像 user 右对齐，需要产品决策。

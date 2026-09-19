# PM Progress

## Index
- [Current Goal](#current-goal)
- [Active Work](#active-work)
- [Recent Completed](#recent-completed)
- [PM Progress Archive](pm-progress-archive/index.md)
- [Known Issues](#known-issues)

## Current Goal
Current installed Runtime Capsule is `sha256:c66acb51dec2412d16855c3e2732ddba8eae584e828dc4deefc37ad051bda928`, selected/externalCurrent/activeLaunch in runtime-launcher control state, installed by recovered restart request `call_2iNOxNhAaM4ukEFkHw0xji1n` (do not repeat the same restart request). Artifact manifest metadata sourceCommit is `20e8e3b8f62d945984a409ef100213f096efd8fc`. Previous Runtime Capsule is `sha256:1cc50bd796e0267b5f5d877c9fa35bc66a8f38206ee83a49d4a2486e473475a1`. Self-debug attached to CDP `127.0.0.1:9223`: listener process is `Root Worker Runtime`, renderer URL points to `c66acb51.../app.asar/dist/index.html`, readyState complete, Electron `37.10.3` UA confirmed, console 0 errors/warnings. Runtime control state shows payload pid `25510` running under launch instance `50557-1789832945626`. `update_plan` accepted `blocked` and multiple non-completed plan states in-turn; current renderer DOM had no RightPanel plan elements visible for label/class assertion. Computer Use direct tool exposure still needs a fresh new-turn model tool-surface check because the restart occurred mid-turn.

## Active Work
- id: plan-tool-minimal-status-model
  owner: /self/owner_dev
  checkout: /Users/bytedance/.morpheus/source_workspace-dev
  branch: fix/plan-tool-rich-status
  task_type: runtime/tool schema + frontend display bugfix
  depends_on: user feedback that plan tool cannot express PM multi-task state; simplified design decision to avoid many PM-specific statuses
  files: codex-rs/protocol/src/plan_tool.rs; codex-rs/protocol/src/prompts/base_instructions/default.md; codex-rs/thread-service prompt mirrors if applicable; apps/root-worker-prototype/src/types.ts; apps/root-worker-prototype/src/components/RightPanel.tsx/tests; apps/root-worker-prototype/src/styles.css
  base_commit: 00939bf8485343e31b1d320b70848b12c93dd3c7
  status: installed_effective
  next_action: none; if desired, verify RightPanel plan label rendering in a future turn/window where Current Plan is visible.
  validation: owner/reviewer approved; `cargo fmt ...` pass with existing nightly rustfmt warning; `cargo test --manifest-path codex-rs/Cargo.toml -p protocol plan_tool -- --nocapture` 2/2; `cargo test --manifest-path codex-rs/Cargo.toml -p app-server test_handle_turn_plan_update_emits_notification_for_v2 -- --nocapture` pass; `pnpm --dir apps/root-worker-prototype exec tsx --test src/components/RightPanel.test.tsx` 59/59; owner `cargo build --manifest-path codex-rs/Cargo.toml -p app-server --bin app-server` pass; PM merge build `cargo build --manifest-path codex-rs/Cargo.toml -p app-server --bin app-server` pass in 61.57s after real feature merge; `git diff --check` pass; old exactly-one/three-state prompt constraints rg-clean. Installed via Runtime Capsule `sha256:c66acb51...` from sourceCommit `20e8e3b8`; self-debug confirmed renderer on new release, console 0 errors/warnings, and `update_plan` accepted `blocked` plus multiple non-completed plan states. `codex-tool-service` focused lib test still has pre-existing test mock/import baseline compile issue outside this task.
  commit: owner `687e0bcd2`; merge `418d78e25`

- id: compact-retained-init-context-visible
  owner: /self/owner_dev
  checkout: /Users/bytedance/.morpheus/source_workspace-dev
  branch: fix/compact-retained-init-context
  task_type: UI/display correction
  depends_on: compact summary inline and Init Context section-item projection
  files: apps/root-worker-prototype/src/lib/conversation.ts; apps/root-worker-prototype/src/lib/conversation.test.ts
  base_commit: 9f0f257a96862401c26f8b816c6bcd2bac4a2afb
  status: installed_effective
  next_action: none.
  validation: owner/reviewer approved; PM design验收 passed; main `pnpm --dir apps/root-worker-prototype exec tsx --test src/lib/conversation.test.ts src/components/Conversation.test.tsx src/lib/conversationVirtualization.test.ts` 126/126; `git diff --check` pass. Installed via Runtime Capsule `sha256:c66acb51...`; self-debug confirmed renderer on new release and console 0 errors/warnings.
  commit: owner `2d5fe2e4b`; merge `2ce7ed63e`

- id: init-context-specific-category-labels
  owner: /self/owner_dev
  checkout: /Users/bytedance/.morpheus/source_workspace-dev
  branch: fix/init-context-specific-labels
  task_type: UI/display typed source classification
  depends_on: installed `split-init-context-into-items`
  files: codex-rs/turn-items/src/lib.rs; codex-rs/turn-items/src/tests.rs
  base_commit: eadb296a83047d44c5072c12369368d60db69e7c
  status: installed_effective
  next_action: none.
  validation: owner/reviewer approved; PM design验收 passed; main `cargo test --manifest-path codex-rs/Cargo.toml -p codex-turn-items injected_context -- --nocapture` 2/2; root-worker focused tests 125/125; `cargo build --manifest-path codex-rs/Cargo.toml -p app-server --bin app-server` pass with linker/future-incompat warnings; `git diff --check` pass. Installed via Runtime Capsule `sha256:c66acb51...`; self-debug confirmed renderer on new release and console 0 errors/warnings.
  commit: owner `65cc5b03c`; merge `6c7682735`

- id: compact-summary-inline-visible
  owner: /self/owner_dev_3
  checkout: /Users/bytedance/.morpheus/source_workspace-dev-3
  branch: fix/compact-summary-inline-visible
  task_type: UI/display correction
  depends_on: installed `compact-marker-non-expandable-ui`
  files: apps/root-worker-prototype/src/components/Conversation.tsx; apps/root-worker-prototype/src/lib/conversationFormatting.ts; apps/root-worker-prototype/src/lib/conversationVirtualization.ts; apps/root-worker-prototype/src/styles.css; focused Conversation tests
  base_commit: eadb296a83047d44c5072c12369368d60db69e7c
  status: installed_effective
  next_action: none.
  validation: owner/reviewer approved; PM design验收 passed; main root-worker focused tests 125/125; app-server debug build pass with linker/future-incompat warnings; `git diff --check` pass. Installed via Runtime Capsule `sha256:c66acb51...`; self-debug confirmed renderer on new release and console 0 errors/warnings.
  commit: owner `8d185ac92`; merge `9f0f257a`

- id: computer-use-mcp-tool-exposure
  owner: /self/owner_dev_2
  checkout: /Users/bytedance/.morpheus/source_workspace-dev-2
  branch: fix/computer-use-mcp-tool-exposure
  task_type: runtime/MCP tool exposure bugfix
  depends_on: installed Computer Use helper/MCP packaging; dev-2 stale dirty formatting/import stash triaged and dropped as not useful
  files: codex-rs/mcp-service/src/tool_exposure.rs; codex-rs/mcp-types/src/lib.rs; codex-rs/mcp-types/src/mcp_config.rs
  base_commit: eadb296a83047d44c5072c12369368d60db69e7c
  status: installed_effective_pending_new_turn_surface_check
  next_action: on next fresh user/model turn, confirm Computer Use MCP tools are directly visible in the model tool surface rather than only discoverable through tool search.
  validation: owner/reviewer approved; PM design验收 passed; main `cargo test --manifest-path codex-rs/Cargo.toml -p mcp-service directly_exposes_computer_use_when_large_tool_sets_are_searchable -- --nocapture` 1/1; `cargo build --manifest-path codex-rs/Cargo.toml -p app-server --bin app-server` pass with existing warnings; `git diff --check` pass. Installed via Runtime Capsule `sha256:c66acb51...`; self-debug confirmed renderer on new release and console 0 errors/warnings. Fresh-turn direct tool-surface verification remains pending because this restart occurred mid-turn.
  commit: owner `6d61a7e7c`; merge `b0e9744b3`

Current checkout allocation: `source_workspace-dev`, `source_workspace-dev-2`, and `source_workspace-dev-3` are idle and synced to the latest main baseline at the last PM synchronization.

## Recent Completed
- id: readme-codex-build-benchmark
  status: merged_no_capsule_required
  owner: /self/owner_dev_3
  reviewer: /self/owner_dev_3/reviewer
  checkout: /Users/bytedance/.morpheus/source_workspace-dev-3
  branch: docs/codex-build-benchmark
  commit: owner `cdf5ad20b`; merge `8573d309e`
  summary: README includes a reproducible single-run Rust app-server debug build benchmark comparing Morpheus `app-server` at `9f0f257a...` and OpenAI Codex reference `codex-app-server` at `78245b47...`, with environment, cold/warm definitions, commands, results, and caveats. PM later adjusted the hot-compile section per user feedback: Morpheus hot sample is the real plan-tool feature merge build `418d78e25` at 61.57s; OpenAI Codex hot sample is now `codex-rs/core/src/lib.rs` timestamp invalidation followed by `codex-app-server` build at 229.62s, so downstream server rebuild/link time is included. The earlier `codex-core`-only 9.12s measurement is documented only as a non-comparable旁注. Pure documentation; no Runtime Capsule needed.
  validation: owner benchmark and reviewer passed; PM inspected README diff and raw log `/tmp/morpheus-codex-build-benchmark.v7STH6/benchmark.log`; PM reran hot samples: Morpheus app-server after feature merge 61.57s, Codex core invalidation -> app-server 229.62s; `git diff --check` pass.

- id: split-init-context-into-items
  status: installed_effective
  owner: /self/owner_dev_3
  reviewer: /self/owner_dev_3/reviewer
  checkout: /Users/bytedance/.morpheus/source_workspace-dev-3
  branch: fix/init-context-multiple-items
  commit: owner `decbd58e4`; merge `6abcc5a6`
  summary: Typed `injectedContext.sections` now project as multiple adjacent Init Context context/tool items (`ctx-id:section:N`) instead of one item with `toolDetailSections`; compact replacement history uses the same section-item projection and context entries are prevented from being re-merged by compact cell grouping.
  validation: owner/reviewer approved; PM design验收 passed against the brief (typed projection, not CSS-only; no regex details splitting; persisted/model-visible history unchanged); main `pnpm --dir apps/root-worker-prototype exec tsx --test src/components/Conversation.test.tsx src/lib/conversation.test.ts src/lib/conversationVirtualization.test.ts src/lib/thread.test.ts` 358/358. Delivered via capsule-only release `sha256:1cc50bd...` from sourceCommit `a4987fa84`, restart `call_rXgzD1RpJ3Z9aGlJCdo5zBG4`; self-debug confirmed installed renderer URL points to `1cc50bd...`, `.tool-detail-sections` count 0, compact summary details/body count 0, and console 0 errors/warnings.

- id: compact-marker-non-expandable-ui
  status: installed_effective
  owner: /self/owner_dev_3
  reviewer: /self/owner_dev_3/reviewer
  commit: owner `c64e67639`; merge `33aaff9cc`
  summary: Compact rows now render as non-expandable `Context compacted` markers; no compact summary `<details>`, summary preview/body, or “Summary available/View summary” appears in the active chat UI.
  validation: owner/reviewer approved; PM design验收 passed; main focused UI/lib tests 114/114 and then combined Init Context tests 121/121; installed via capsule-only release `sha256:3e6269...`; self-debug DOM confirmed compact summary details/body count 0 and console 0 errors/warnings.

- id: split-init-context-display
  status: installed_effective
  owner: /self/owner_dev_3
  reviewer: /self/owner_dev_3/reviewer
  commit: owner `ac04bd218`; merge `924c1a598`
  summary: Init Context remains one row/event but carries typed `toolDetailSections`; ToolRow details render section labels/text separately while preserving `toolDetails` as compatibility/search fallback.
  validation: owner/reviewer approved after virtualization height double-count fix; PM design验收 passed; main `pnpm --dir apps/root-worker-prototype exec tsx --test src/components/Conversation.test.tsx src/lib/conversation.test.ts src/lib/conversationVirtualization.test.ts` 121/121; installed via capsule-only release `sha256:3e6269...`; self-debug confirmed new renderer bundle and zero console errors/warnings.

- id: capsule-only-runtime-delivery
  status: installed_effective
  owner: /self/owner_dev
  reviewer: /self/owner_dev/reviewer
  commit: owner `572d158d305b5a1d6a2f39aa7db90733e0168b7f`; merge `71e628861`
  summary: Added explicit `package:root-worker-prototype:mac:capsule` path that builds a complete Runtime Capsule into `runtime-launcher/incoming/<activationId>` without rebuilding the outer Launcher or DMG; full mac packaging still reuses the capsule stage.
  validation: owner/reviewer approved; PM design验收 passed; main script tests 9/9; actual delivery used `pnpm package:root-worker-prototype:mac:capsule`, produced `sha256:3e6269...`, and recovered restart `call_74YbmRKaOr3SkSr4SJYNhgXn` selected it as externalCurrent/activeLaunch. No DMG/full outer app packaging was used for this delivery.

- id: file-tree-open-normal-preview
  status: installed_effective
  owner: /self/owner_dev
  reviewer: /self/owner_dev/reviewer
  checkout: /Users/bytedance/.morpheus/source_workspace-dev
  branch: fix/file-tree-opens-file-preview
  commit: owner `d3a5ab0c8b8a9e14b1e8f9226f3fdec57fcd2cbf`; merge `53717f8991d08022f6db1d14c4f1a0b3e21c1bc3`
  summary: File tree file clicks now open normal file preview; Git panel staged/unstaged/commit file rows remain the diff-entry points. Removed file-tree-to-Git-diff resolver/status lookup.
  validation: owner/reviewer approved; PM design验收 passed; `pnpm --dir apps/root-worker-prototype exec tsx --test src/components/RightPanel.test.tsx` 59/59; `git diff --check HEAD~1..HEAD` pass. Delivered in Runtime Capsule `sha256:bb7f0bab0d56b639995f6ead7d38441c26d3f212dcb590e9fdfca40ed6a033f3`; self-debug installed renderer URL points to `bb7f0bab...`, console 0 errors/warnings.

- id: computer-use-helper-signed-equivalence
  status: installed_effective
  owner: /self/owner_dev_3
  reviewer: /self/owner_dev_3/reviewer
  checkout: /Users/bytedance/.morpheus/source_workspace-dev-3
  branch: fix/computer-use-helper-signed-equivalence
  commit: owner `c108505c732a66328b4af4fb4974e204dcb3f0cd`; merge `e79fb2ecc26e2fa39d96a2bcc11b2356c5b6237b`
  summary: Stable Computer Use helper equivalence now normalizes Mach-O code-signature-only differences instead of raw hashing signed executable bytes, while still detecting real code/resource changes.
  validation: owner/reviewer approved; PM design验收 passed; main `node --test apps/root-worker-prototype/electron/installedArtifactUpdate.test.cjs apps/root-worker-prototype/electron/appServerClient.test.cjs` 85/85; package/restart installed `sha256:c1ea6473...`; explicit current-release materialization dry-check returned `status:"unchanged"` without changing inode/mtime. After user authorization, MCP `computer.permissions_status(includeObservation:true, includePerception:false)` reported Screen Recording granted, Accessibility true, stablePermissionSubject true. Subsequent UI Capsule delivery did not rewrite the helper.

- id: readme-project-comparison
  status: merged_no_capsule_required
  commit: `17bf5645c Document Morpheus project comparison`
  summary: README rewritten in Chinese with Morpheus overview and comparison against OpenAI Codex, Claude Code Best, and DeepSeek Harness/dsh; stable reference source locations documented. Pure documentation change; no Runtime Capsule needed.

## Known Issues
- 2026-09-19 Computer Use stable helper no-rewrite keeps `Contents/Resources/payload-electron-path` unchanged by design to avoid touching the stable helper bundle. After the `bb7f0bab...` delivery, MCP server diagnostics still show `execPath` from previous artifact `c1ea6473...` while the main Runtime is `bb7f0bab...`. This is currently safe because `c1ea6473...` is externalPrevious, but future artifact GC could break the stable helper MCP server if the referenced previous payload is removed. Need a follow-up design for a stable current-runtime indirection or non-TCC-breaking payload pointer update.
- 2026-09-19 `.morpheus/config.toml` remains dirty from user/runtime config changes and must not be committed accidentally.
- Older completed entries and stale known issues are archived in [PM Progress Archive](pm-progress-archive/index.md).

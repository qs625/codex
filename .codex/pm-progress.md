# PM Progress

## Index
- [Current Goal](#current-goal)
- [Active Work](#active-work)
- [Recent Completed](#recent-completed)
- [PM Progress Archive](pm-progress-archive/index.md)
- [Known Issues](#known-issues)

## Current Goal
Deliver the current UI/runtime tranche together: Context Window should show all split tool buckets including zero-value categories, `poll_event` / `poll_external_event` should expose only bounded wake category metadata to models, and the root-worker main workspace should become an editor-style tab workspace where each concrete opened object/session is a tab (for example multiple conversations, terminal sessions, and files), not four fixed surface tabs for Conversation / Files / Terminal / Browser. Context Window and poll_event changes are already installed via Runtime Capsule `sha256:23a8bdfe02712773a34a86bed8f7050c28817b628b14238f6781ec92e9115749`; that same capsule also installed the now-superseded fixed surface-tab UI, which the user clarified is not the intended design. Do not call restart again for `call_jMq3DzKPxwulxHPb2JgpSxAV`; implement a corrective UI tranche and then build/install a fresh capsule.

## Active Work
- id: workspace-tabbed-shell-ui
  status: superseded_by_editor_style_tab_clarification
  owner: /self/owner_dev_3
  reviewer: /self/owner_dev_3/reviewer
  checkout: /Users/bytedance/.morpheus/source_workspace-dev-3
  branch: feat/workspace-tabbed-shell
  task_type: frontend_ui_shell_feature
  depends_on: main `8f6afc3472482b0c8b0cf85c80890c562686d28b`; intermediate built capsule `sha256:f3951bda3b5e7d16dc4ef5f1813aee4a549944d61bd63caaf6c22a8e22ddbe49` intentionally not installed
  files: `apps/root-worker-prototype/src/App.tsx`, `apps/root-worker-prototype/src/components/Conversation.tsx`, `apps/root-worker-prototype/src/components/RightPanel.tsx`, `apps/root-worker-prototype/src/components/TerminalPanel.tsx`, `apps/root-worker-prototype/src/styles.css`, frontend tests
  base_commit: `8f6afc3472482b0c8b0cf85c80890c562686d28b`
  next_action: Do not continue validating this fixed surface-tab implementation as final. User clarified the desired UI is editor-style concrete-object tabs: each opened conversation, terminal session, file/preview, etc. should be its own central tab, similar to an editor with many open files. Dispatch corrective tranche to owner_dev_3.
  validation: Owner implemented first-slice workspace tabs and fixed reviewer findings. PM design/diff validation passed: Conversation/File/Terminal/Browser are same-level workspace tabs; Conversation app-internal header was removed and thread metadata/run config/cwd/search moved into the composer area; Files/Terminal/Browser reuse real existing panels; Browser native view is hidden while Browser is inactive or overlays/resizing are active; Terminal is only mounted while the Terminal workspace tab is active; RightPanel is reduced to auxiliary Thread Analysis/Git/Workflow surfaces in tabbed mode; no close/new workspace-tab buttons or placeholder/no-op Conversation buttons remain. Owner and PM both ran `pnpm --dir apps/root-worker-prototype test src/components/Panels.test.tsx src/components/RightPanel.test.tsx src/components/TerminalPanel.test.tsx src/lib/workspaceTabs.test.ts src/lib/rightPanelView.test.ts src/lib/filePreviewMemory.test.ts src/lib/conversation.test.ts` with 190 passed. Owner and PM `pnpm --dir apps/root-worker-prototype build` passed with existing Vite chunk/xterm dynamic import and chunk-size warnings. PM `git diff --check` passed before and after merge. Runtime Capsule build produced `sha256:23a8bdfe02712773a34a86bed8f7050c28817b628b14238f6781ec92e9115749` from sourceCommit `a9bff923c6cc0437f3326fa94925a15e5c767804`; build warnings were existing Vite chunk/xterm, app-server unused/dead-code, Rust future-incompat, and Swift deprecation/unused warnings.
  commit: owner `f0da7aad1340ffb56ad68e0de9d7f368d6bfb52d`; PM progress `8edc9bc57`; merge `120c8ee4df89efe74e02bf283ffa5a6771cabaf6`; progress/capsule source `a9bff923c6cc0437f3326fa94925a15e5c767804`; installed capsule `sha256:23a8bdfe02712773a34a86bed8f7050c28817b628b14238f6781ec92e9115749`; restart `call_jMq3DzKPxwulxHPb2JgpSxAV`; superseded by user clarification

- id: workspace-editor-style-object-tabs
  status: capsule_built_pending_restart
  owner: /self/owner_dev_3
  reviewer: /self/owner_dev_3/reviewer
  checkout: /Users/bytedance/.morpheus/source_workspace-dev-3
  branch: feat/workspace-editor-object-tabs
  task_type: frontend_ui_shell_feature
  depends_on: main `ceca6c329147cd019a7d35d502d9ef6069e1cd43`; supersedes fixed surface-tab UI installed in `sha256:23a8bdfe02712773a34a86bed8f7050c28817b628b14238f6781ec92e9115749`
  files: `apps/root-worker-prototype/src/App.tsx`, `apps/root-worker-prototype/src/components/Panels.tsx`, `apps/root-worker-prototype/src/components/RightPanel.tsx`, `apps/root-worker-prototype/src/components/TerminalPanel.tsx`, `apps/root-worker-prototype/src/lib/*workspace*tab*`, `apps/root-worker-prototype/src/styles.css`, frontend tests
  base_commit: `ceca6c329147cd019a7d35d502d9ef6069e1cd43`
  next_action: Request one complete Runtime Capsule restart to install `sha256:b04fa10407b42602d3978a0671d4e6fa4a3f0f3193b822dafdd01c42e83df80a`, then do installed self-debug validation.
  validation: Owner implemented corrective tranche and reviewer approved after storage/stale-tab fixes. PM design/diff validation passed against user clarifications: fixed `Conversation / Files / Terminal / Browser` surface tabs removed; central tabs are concrete conversation/thread object tabs with status dot, title/path, subtitle/presence; File Preview / Terminal / Browser remain default right-panel entries; no fake drag-to-middle affordance for File/Terminal/Browser; Conversation fills active workspace panel; Conversation internal right border and composer top border removed so only real splitter/resizer expresses middle/right boundary; resizer hit target widened from 1px to 6px; macOS window uses `titleBarStyle: "hiddenInset"` and preserves native frame/traffic lights. PM validation on dev and main: `node --import tsx --test apps/root-worker-prototype/src/components/Panels.test.tsx apps/root-worker-prototype/src/components/RightPanel.test.tsx apps/root-worker-prototype/src/components/TerminalPanel.test.tsx apps/root-worker-prototype/src/lib/workspaceTabs.test.ts apps/root-worker-prototype/src/lib/rightPanelView.test.ts apps/root-worker-prototype/src/lib/filePreviewMemory.test.ts apps/root-worker-prototype/src/lib/conversation.test.ts apps/root-worker-prototype/electron/windowChrome.test.cjs` passed 191 tests; `git diff --check` passed; `pnpm --dir apps/root-worker-prototype build` passed with existing Vite chunk/xterm warnings. Runtime Capsule build produced `sha256:b04fa10407b42602d3978a0671d4e6fa4a3f0f3193b822dafdd01c42e83df80a` from sourceCommit `64bd3bea19bf42fba68ca3245ef607dfd6975223`.
  commit: owner `1fe8575dc086c39c093add24046120fde4779a0e`; PM progress `23b7dc156`; merge `a34dc5336ce76f9c6029f3a27be48fb654a5c34b`; progress/capsule source `64bd3bea19bf42fba68ca3245ef607dfd6975223`; capsule `sha256:b04fa10407b42602d3978a0671d4e6fa4a3f0f3193b822dafdd01c42e83df80a`

- id: poll-event-category-only-result
  status: capsule_built_not_installed_superseded_by_ui_tranche
  owner: /self/owner_dev_2
  reviewer: /self/owner_dev_2/reviewer
  checkout: /Users/bytedance/.morpheus/source_workspace-dev-2
  branch: fix/poll-event-category-only-result
  task_type: runtime_tool_surface_change
  depends_on: main `c87b288a26bb6eedfa04541ae48cf7b5c92c5021`
  files: `codex-rs/thread-service/src/session/pending_input.rs`, `codex-rs/thread-service/src/session/thread_wait.rs`, `codex-rs/tool-service/src/planning/tool_specs/multi_agents.rs`, poll_event tests in `codex-rs/thread-service`, `codex-rs/tool-service`, and app-server display tests as needed
  base_commit: `c87b288a26bb6eedfa04541ae48cf7b5c92c5021`
  next_action: Runtime Capsule `sha256:f3951bda3b5e7d16dc4ef5f1813aee4a549944d61bd63caaf6c22a8e22ddbe49` was built from sourceCommit `8f6afc3472482b0c8b0cf85c80890c562686d28b`, but user immediately requested a larger UI shell/tab redesign before restart. Do not install this intermediate capsule unless explicitly needed; build a fresh capsule after the UI tranche and install that.
  validation: Owner/reviewer completed. Owner reports `ThreadPollEventResult` now exposes `sourceCategory` and skips serializing internal `event/events`; native/external poll paths fill coarse categories (`command`, `subagent`, `user_input`, `queued_input`, `async_input`); schema/description remove payload contract; workflow `agent.wait()` keeps internal completion payload; frontend poll_event summary prefers category. Owner and PM validation: `cargo test --manifest-path codex-rs/Cargo.toml -p thread-service poll_event` 10 passed; `cargo test --manifest-path codex-rs/Cargo.toml -p thread-service external_tool_call_poll_external_event` 4 passed; `cargo test --manifest-path codex-rs/Cargo.toml -p codex-workflow poll_event` 1 passed; `pnpm --dir apps/root-worker-prototype exec tsx --test src/lib/conversation.test.ts` 76 passed; `cargo build --manifest-path codex-rs/Cargo.toml -p app-server --bin app-server` passed with existing linker/future-incompat warnings; `git diff --check` passed. Owner `rustfmt --check` on touched Rust files passed. `codex-tool-service poll_event` lib test remains blocked by existing test harness compile debt.
  commit: owner `ae49b164e01a50c24b1ffb4146b37e6cc394a5a3`; merge `35ef113dc`

- id: context-window-show-zero-categories
  status: capsule_built_not_installed_superseded_by_ui_tranche
  owner: /self/owner_dev
  reviewer: /self/owner_dev/reviewer
  checkout: /Users/bytedance/.morpheus/source_workspace-dev
  branch: fix/context-window-show-zero-categories
  task_type: frontend_ui_behavior
  depends_on: main `c87b288a26bb6eedfa04541ae48cf7b5c92c5021`
  files: `apps/root-worker-prototype/src/lib/contextUsage.ts`, `apps/root-worker-prototype/src/lib/contextUsage.test.ts`, `apps/root-worker-prototype/src/components/RightPanel.test.tsx`
  base_commit: `c87b288a26bb6eedfa04541ae48cf7b5c92c5021`
  next_action: Runtime Capsule `sha256:f3951bda3b5e7d16dc4ef5f1813aee4a549944d61bd63caaf6c22a8e22ddbe49` includes this change but was not installed because user immediately requested a larger UI shell/tab redesign before restart. Deliver in a fresh capsule after the UI tranche.
  validation: Owner/reviewer completed. Owner and PM both ran `pnpm --dir apps/root-worker-prototype test src/lib/contextUsage.test.ts src/components/RightPanel.test.tsx` passed 73 tests; PM `git diff --check` passed. Implementation keeps backend accounting unchanged, shows all split tool buckets including zero-value `Inter-Agent`, keeps aggregate `Tool Inputs & Results` hidden in split mode, and preserves fallback when breakdown/toolCalls cannot be split.
  commit: owner `8cd14123ad4f`; merge `f2f6060f8`

- id: compact-project-instruction-files-installed-regression
  status: installed_effective_pending_next_compact_observation
  owner: /self/owner_dev
  reviewer: /self/owner_dev/reviewer
  checkout: /Users/bytedance/.morpheus/source_workspace-dev
  branch: fix/compact-project-instruction-files
  task_type: runtime_context_bugfix
  depends_on: installed delivery `sha256:2f319ed799683ce0848d4b2c5c1cf9b3b4aa77e98c29ad12426e220f0aa47c43`
  files: `codex-rs/config/src/local_loader/mod.rs`, `codex-rs/thread-service/src/session/tests/context_and_history.rs`, `codex-rs/app-server/src/request_processors*`
  base_commit: `effc84f3ca474acaf77d237368eb68f58867372f`
  next_action: No further compatibility work for old compact segments. On the next natural compact, verify the new flat contract in installed UI: marker-only `ContextCompaction`, independent compact summary AgentMessage, and independent Init Context items. Keep `apps/root-worker-prototype/dist-seed-capsule/` untracked and do not commit generated output.
  validation: owner/reviewer root-caused and fixed two regressions: project `instruction_files = ["instructions/..."]` were incorrectly resolved against project root instead of `.morpheus/config.toml` directory, and persisted compact replacement history was not expanded for read/list/startup display. PM built Runtime Capsule `sha256:f68752167e7c8cce3c9cb1cd57cba7c40886c6bd942b91c2fd48821b8ea6696a`; restart `call_Q4jwcbu7yd4Qf24Msf8k0lr4` recovered/completed; installed validation then showed Telebot Init Context present but compact summary still missing. Real Telebot rollout `compact-000025.jsonl` showed summary is stored in `CompactedItem.message`, not as a nested replacement-history AgentMessage. User required a clean flat display/API contract with marker-only `ContextCompaction` and independent summary/Init Context items, without complex nested `replacementHistory` compatibility. Owner final fix removed display/API/frontend nested replacement-history contract and preserved only internal core/model `CompactedItem.replacement_history`; reviewer approved. PM merged owner `7ba7880bd` via merge `eca096f67`, cleaned validation warnings in `a466302ff`, and validated main: `cargo test --manifest-path codex-rs/Cargo.toml -p app-server-protocol --lib` 223 passed; `cargo test --manifest-path codex-rs/Cargo.toml -p thread-history compact` 13 passed; `cargo test --manifest-path codex-rs/Cargo.toml -p app-server --lib compact` 25 passed; `cargo build --manifest-path codex-rs/Cargo.toml -p app-server --bin app-server` passed with existing linker/future-incompat warnings; `pnpm --dir apps/root-worker-prototype test src/components/Conversation.test.tsx src/lib/conversation.test.ts src/lib/conversationPresentation.test.ts src/lib/conversationSearch.test.ts src/lib/conversationVirtualization.test.ts src/lib/thread.test.ts src/lib/threadAnalysis.test.ts` 414 passed; `git diff --check` passed; acceptance `rg "ContextCompactionReplacementItem|replacementHistory|replacement_history" codex-rs/app-server-protocol/schema codex-rs/app-server-protocol/src/protocol/item codex-rs/app-server/README.md apps/root-worker-prototype/src || true` returned empty. Runtime Capsule build produced `sha256:92094a3cc9038a3c22551873332eeb9f3d535b0e2f038e9b157f39a88b760e92` from sourceCommit `3b3394cbb627f37bdd3998fa7b6711b59db067fc`; restart `call_7VJnjb3EsYMNPorGiv1livKE` completed. Installed validation: `runtime-launcher/control.json` externalCurrent/selected/activeLaunch all point to `92094a3c...`, payload pid `70328`, app-server pid `70332`, renderer URL points to `92094a3c.../app.asar/dist/index.html`, Playwright/CDP attached to Electron UA, console 0 errors/warnings. Old Telebot compact segment validation was not pursued further because user accepted that old data/cache compatibility is not required; next natural compact will validate the new flat contract.
  commit: owner initial `83ae63e421c338775cac275124a408bfec5b248c`; merge `50e0f2816a0ff8185d734389445ba407a1ed09dc`; prior installed capsule `sha256:f68752167e7c8cce3c9cb1cd57cba7c40886c6bd942b91c2fd48821b8ea6696a`; owner flattened fix `7ba7880bd`; merge `eca096f67`; PM cleanup `a466302ff`; progress `3b3394cbb`; installed capsule `sha256:92094a3cc9038a3c22551873332eeb9f3d535b0e2f038e9b157f39a88b760e92`; restart `call_7VJnjb3EsYMNPorGiv1livKE`; next compact observation pending

## Recent Completed
- id: compact-project-instruction-files
  status: installed_effective
  owner: /self/owner_dev
  reviewer: /self/owner_dev/reviewer
  checkout: /Users/bytedance/.morpheus/source_workspace-dev
  branch: fix/compact-project-instruction-files
  commit: owner `c12146ef52f0ba74206c2abe12d040954fad7446`; merge `d850cd0738d6a056755556512ce3d988e8d63ce7`
  summary: Compact fresh init context now reloads project-aware config for current cwd and merges refreshed project/user `instruction_files` into runtime config before rendering user instructions. Project-level instruction files remain explicit via `.morpheus/config.toml`; project instruction directories are not auto-scanned.
  validation: owner `cargo test --manifest-path codex-rs/Cargo.toml -p thread-service fresh_compact_initial_context` 5 passed and `cargo build --manifest-path codex-rs/Cargo.toml -p app-server --bin app-server` passed; reviewer approved; PM `cargo test --manifest-path codex-rs/Cargo.toml -p thread-service fresh_compact_initial_context -- --nocapture` 5 passed; PM `cargo test --manifest-path codex-rs/Cargo.toml -p thread-service ordinary_context_update_does_not_reload_available_skill_body -- --nocapture` passed; PM `cargo build --manifest-path codex-rs/Cargo.toml -p app-server --bin app-server` passed with existing linker/future-incompat warnings. Delivered in Runtime Capsule `sha256:2f319ed799683ce0848d4b2c5c1cf9b3b4aa77e98c29ad12426e220f0aa47c43` from sourceCommit `e4d48c4143ecb79c165b0e0d0feb97bc628c79e9`; restart request `call_qJve0JO8odiQ2vdSEe0isffw` completed. Installed verification: `runtime-launcher/control.json` externalCurrent/selected/activeLaunch all point to `2f319ed...`; payload pid `27553`; installed app-server pid `27563`; renderer URL points to `2f319ed.../app.asar/dist/index.html`; Electron console 0 errors / 0 warnings.

- id: init-context-skill-body-hygiene
  status: installed_effective
  owner: /self/owner_main
  reviewer: n/a
  checkout: /Users/bytedance/.morpheus/source_workspace
  commit: `957ce05e91d1d2d4245fee0ccfe326a70f110eaa`
  summary: Init Context, reference snapshots, and ordinary context diffs no longer read or inject available/implicit skills' `SKILL.md` bodies. Available skills remain visible as metadata; full skill bodies are reserved for explicit skill injection/trigger paths.
  validation: PM removed the skill-body injection helper and updated regression tests; `cargo test --manifest-path codex-rs/Cargo.toml -p thread-service ordinary_context_update_does_not_reload_available_skill_body -- --nocapture` passed; included in Runtime Capsule `sha256:2f319ed799683ce0848d4b2c5c1cf9b3b4aa77e98c29ad12426e220f0aa47c43` and installed verification above.

- id: followup-tool-item-hide-stale-agent-state
  status: installed_effective
  owner: /self/owner_dev_2
  reviewer: /self/owner_dev_2/reviewer
  checkout: /Users/bytedance/.morpheus/source_workspace-dev-2
  branch: fix/init-context-baseline-missing-root-cause
  commit: owner `a51d80a8fd9413c96f1e8338fe5b412866162563`; merge `9d8f24efd50e42bee94cea69819783e401a1b1c6`
  summary: Tool item details now hide stale `Agent States` details for `sendInput`/`resumeAgent` while retaining spawn/list/wait/close result display where current state is an actual tool result.
  validation: owner `pnpm --dir apps/root-worker-prototype exec tsx --test src/lib/conversation.test.ts` 84 passed; `pnpm --dir apps/root-worker-prototype exec tsx --test src/lib/conversationPresentation.test.ts` 12 passed; `git diff --check` passed; reviewer approved; PM diff/design验收 passed. Delivered in Runtime Capsule `sha256:2f319ed799683ce0848d4b2c5c1cf9b3b4aa77e98c29ad12426e220f0aa47c43` and installed verification above.

- id: inter-agent-tool-real-parameter-display
  status: installed_effective
  owner: /self/owner_dev_2
  reviewer: /self/owner_dev_2/reviewer
  checkout: /Users/bytedance/.morpheus/source_workspace-dev-2
  branch: fix/followup-tool-item-stale-agent-state
  commit: owner `309bc733ed460c597cdef3a94f05c0d38f4e92af`; merge `d5f911e6c8d51860eaf414e86ad9a46833aad88d`
  summary: Inter-agent/collab tool items now show real, reconstructable tool parameters by tool kind, e.g. `target`/`content`, `message`/`model`/`reasoning_effort`, `timeout_ms`, and `path_prefix`; they no longer force everything into generic `Sender`/`Receivers`/`Prompt`/`Agent States`, and they do not invent parameters absent from the stored projection.
  validation: owner `pnpm --dir apps/root-worker-prototype exec tsx --test src/lib/conversation.test.ts` 85 passed; `pnpm --dir apps/root-worker-prototype exec tsx --test src/lib/conversationPresentation.test.ts` 12 passed; `git diff --check` passed; reviewer approved after agent_type non-invention fix; PM diff/design验收 passed. Delivered in Runtime Capsule `sha256:2f319ed799683ce0848d4b2c5c1cf9b3b4aa77e98c29ad12426e220f0aa47c43` and installed verification above.

- id: browser-cdp-direct-target-creation
  status: installed_effective
  owner: /self/owner_dev_2
  reviewer: /self/owner_dev_2/reviewer
  checkout: /Users/bytedance/.morpheus/source_workspace-dev-2
  branch: fix/browser-cdp-direct-target-creation
  commit: merged through `fb9d8813c1ce5a49820071ee55cea000266cb832`
  summary: Browser CDP compatibility proxy now supports direct target creation for `/json/new` and browser-level websocket `Target.createTarget` without hanging, and the follow-up native BrowserView lifecycle fix prevents direct-created tabs from forcing Browser panel attach/raise or full-window overlay while another right panel is active.
  validation: focused owner/reviewer tests passed before merge; installed Runtime Capsule `sha256:16f2fe9c704e42d438f2604afc904e60f05c4017a56f212df8a96915e2d02562` is running from main `fb9d8813c1ce5a49820071ee55cea000266cb832` after restart request `call_EvYNHCj9ESFHGrDc4zOyyTqd`. Installed CDP validation: `/json/new` returned 200 target info for `https://example.org/?morpheus_json_new_installed=1` with target id `A7913E23359466A9AAA9338478FED399`; browser websocket `Target.createTarget` returned target id `738288D2225E78DBD367E65521350C36` for `https://example.net/?morpheus_create_target_installed=1`; visible Browser content was bounded to the right panel (Browser content/control x `1049..1474`, rail x `1474..1520`, app width `1520`); with Terminal active, direct `Target.createTarget` returned target id `34CAEC8CCB703CD0889E7FF2BD9D2464` for `https://example.com/?morpheus_hidden_panel_installed=1` while Terminal stayed active, Browser stayed inactive, and main renderer DOM did not show Example Domain or the hidden URL. Cleanup closed matching test targets; remaining `morpheus_(json_new|create_target|hidden_panel)_installed=1` targets are `[]`. OS screenshot proof unavailable because `computer_use.observe` failed with macOS `screencapture ... could not create image from display`; Playwright/CDP/DOM evidence was used instead.
  workspace: tracked working tree clean after PM stash isolation; unrelated tracked dirt remains preserved in `stash@{Tue Sep 22 18:35:13 2026}: On main: pm isolate dirty tracked files before runtime capsule 2026-09-22`; generated `apps/root-worker-prototype/dist-seed-capsule/` remains untracked and must not be committed.

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
- 2026-09-22 PM isolated unrelated tracked dirt into `stash@{Tue Sep 22 18:35:13 2026}: On main: pm isolate dirty tracked files before runtime capsule 2026-09-22`; do not blindly pop it because it contains mixed non-Browser changes. `apps/root-worker-prototype/dist-seed-capsule/` is generated output and remains untracked; do not commit it.
- Older completed entries and stale known issues are archived in [PM Progress Archive](pm-progress-archive/index.md).

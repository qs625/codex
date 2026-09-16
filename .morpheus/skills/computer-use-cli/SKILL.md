---
name: computer-use-cli
description: Use the Morpheus-owned Computer Use CLI prototype for read-only macOS desktop inspection, backend/session boundary experiments, bounded app state reads, approval handling, and session/repl design constraints for future GUI actions.
---

# Computer Use CLI

Use this skill when a task needs desktop app inspection through the Morpheus-owned Computer Use CLI prototype.

## Core Rules

- Prefer the Morpheus-owned default runtime path first: `scripts/morpheus-computer-use.mjs`.
- Treat bundled ChatGPT/Codex Computer Use only as a short-term `bundled-sky` backend adapter and behavioral reference.
- MVP commands are read-only: `list-apps` and `get-app-state`.
- Do not implement or call click, drag, type, paste, keypress, upload, delete, send, or other side-effect actions from this skill.
- Do not patch ChatGPT.app, bundled Codex files, or `~/.codex/plugins/cache`.
- Keep output bounded. Use `--preview-chars` unless the caller explicitly needs full JSON.
- For duplicate app identifiers, pass a full `.app` path with `--app`.

## Quick Start

Resolve the skill directory, then run:

```bash
node scripts/morpheus-computer-use.mjs --help
node scripts/morpheus-computer-use.mjs list-apps --json --preview-chars 12000
node scripts/morpheus-computer-use.mjs get-app-state --app "Finder" --json --preview-chars 12000 --no-screenshot
node scripts/morpheus-computer-use.mjs list-apps --backend bundled-sky --json --preview-chars 12000
```

`get-app-state` defaults to a full tree request with `disableDiff=true`, because one-shot CLI commands do not have a reliable previous AX diff baseline. Use `--diff` only inside a deliberate continuous session/batch workflow. The current `macos-native` MVP does not capture screenshots; `--no-screenshot` documents that expectation.

## Backend Model

- `macos-native` is the default self-owned read-only backend. It currently lists apps through macOS metadata and running process inspection, and reads a bounded Accessibility tree through the local macOS automation surface.
- `bundled-sky` is an adapter for the bundled trusted `node_repl` + Computer Use runtime. Use it for comparison or short-term smoke only; do not design new skill behavior around private bundled paths.
- Future backends should fit the same command/session shape, especially a long-lived session backend for click/type/drag workflows.

## Approval Policy

- `list-apps` should not need app approval.
- `get-app-state` may require macOS Accessibility/Automation permission, depending on the backend and target app.
- `bundled-sky get-app-state` may trigger a Computer Use app approval. Only pass `--allow-read-approval session` when the user has explicitly asked for this read-only inspection.
- If approval is unavailable or declined, report the error and stop.
- Side-effect operations require a future explicit confirmation boundary and are intentionally absent from the MVP CLI.

## Session Model

Single CLI commands are suitable for read-only inspection.

Future GUI actions must not be split across many stateless shell commands: that would lose approval/session state, previous screenshot or AX diff baseline, current app/window state, mouse drag continuity, and operation context.

Future action support must use one of:

- `server`: one long-lived process holding node_repl, approvals, and diff baseline, with subcommands talking to it over a socket.
- `repl`: interactive session for continuous observation and action.
- `run-script` or batch command: all dependent actions happen inside one trusted runtime process.

Until one of these exists, keep click/move/drag/type/paste disabled.

## Troubleshooting

- Native permission failure: grant Accessibility/Automation permission to the invoking host, or retry with a safe running app.
- Bundled runtime missing: verify `/Applications/ChatGPT.app/Contents/Resources/cua_node/bin/node_repl`, or set `MORPHEUS_CUA_NODE_REPL`.
- Bundled modules missing: verify `/Applications/ChatGPT.app/Contents/Resources/cua_node/lib/node_modules`, or set `MORPHEUS_CUA_NODE_MODULES`.
- Ambiguous app: rerun `list-apps` and pass a full path or more specific identifier.
- Huge output: lower `--preview-chars` or omit screenshots with `--no-screenshot`.

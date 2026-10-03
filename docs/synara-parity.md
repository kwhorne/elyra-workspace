# Synara parity: what Elyra Workspace is missing

Gap analysis of Elyra Workspace against Synara (`~/Downloads/synara-main`), compiled
2026-10-02 from Synara's user docs (`apps/marketing/content/docs`, `docs/`), web UI
(`apps/web/src`), server and contracts (`apps/server/src`, `packages/contracts/src`)
and desktop shell (`apps/desktop/src`).

Effort: **S** ≈ a day or less, **M** ≈ a few days, **L** ≈ a week+, **XL** ≈ several weeks.
Beta-only features in Synara (Hubs, Tasks, Inbox, Computer use) are marked *(beta)*.

## Where we stand

Synara is ~670k lines of TypeScript: a WebSocket RPC server with an event-sourced
orchestration engine (128 migrations, ~90 tables), a React client, an Electron shell and
10 provider adapters. Elyra is a single Rust process.

**Elyra has today:** projects (add, remove, change folder) · threads (create, archive,
title from first prompt, tabs restored) · Claude Code (streaming text/thinking, tool calls
with results, approvals incl. allow-for-session, interrupt, model and permission mode,
resume) · local or managed worktree per thread · Changes panel (status, unified diff,
discard file, commit all, push) · multi-tab terminal with full TUI support · Settings
window with search and reset (themes, fonts, terminal) · About · app icon and bundle.

**Biggest gaps, in order of daily impact:**

1. Conversation features around the agent: structured questions, attachments and
   mentions, slash commands, queue/steer, subagents, plans and todo cards, usage meter.
2. Thread management: rename, delete, unarchive, pin, unread/attention, notifications.
3. Git beyond commit-all: staging, branches, pull, generated messages, PRs, checkpoints.
4. Navigation: command palette, file search, explorer/editor, split and side chats.
5. More providers (Codex, Cursor/ACP family, OpenCode, Pi, Antigravity) behind a
   provider-neutral runtime.
6. Orchestration: automations, Kanban/Tasks, agent gateway MCP, handoff and fork, import.

## Progress

| Phase | Status |
|---|---|
| Architecture 1 — provider trait and neutral events | ✅ `AgentSession`, `ProviderEvent`, Claude + **Elyra** adapters |
| Phase 1 — Conversation essentials | ✅ all 13 items (2026-10-03) |
| Phase 2 — Threads, attention, app shell | ✅ (2026-10-03) — auto-updater is a release check with download link; installing updates in place needs signed releases |
| Phase 3 — Git, diff and review | ✅ (2026-10-03) |
| Phase 4 — Navigation, files and layout | ✅ (2026-10-03) — not yet: PDF preview, LSP, in-app shortcut recorder, theme editor, time format; the right dock keeps fixed tabs (Changes, PR, Files, Context, Terminal, Side chat) |
| Phase 5 — Providers | ✅ (2026-10-03) — shared ACP runtime (Gemini CLI, Cursor Agent, OpenCode, custom ACP command), Pi via the Elyra RPC adapter, provider settings (enable, executable, arguments, env, accounts, version, sign-in), starred model presets, handoff, fork, Claude Code import, Claude subagent mentions. Postponed: **Codex**. Not yet: Devin/Factory Droid/Grok presets (use the custom ACP command), stop-subagent control, CLI update checks, provider ordering, import from Codex/Elyra history |
| Phase 6 — Orchestration and automation | ✅ (2026-10-03) — agent gateway (MCP over local HTTP, injected into Claude Code and ACP agents, off by default), external MCP with paired read-only/full clients, stdio bridge (`elyra mcp-bridge`) and audit log, automations (once/interval/daily/weekdays/weekly/cron+tz, new or same thread, stop phrase, max runs, failure policy, run history), task board with drag and drop that follows the chat, thread goals with a turn budget, debug mode, thread ZIP export, usage statistics with a 26-week heatmap. Not yet: Hubs/Studio, Inbox, automations while the app is closed (menu-bar/login item), token heatmap (providers report cost and time, not always tokens) |

## Roadmap

### Phase 1 — Conversation essentials ✅

| # | Feature | Notes | Effort |
|---|---|---|---|
| 1 | **Question cards** (AskUserQuestion) | Claude sends it as `can_use_tool`; render options/multi-select, answer via `updatedInput` | S |
| 2 | **Plan card** (ExitPlanMode) | "Plan ready" card with Approve/Keep planning; switch mode on approve | S |
| 3 | **Todo card** (TodoWrite) | Live task list with progress instead of a raw tool row | S |
| 4 | **Subagents** (Task/Agent) | Group child tool calls under the subagent row; status, stop | M |
| 5 | **Attachments** | Paste/drop images and files, chips, send as content blocks | M |
| 6 | **@ mentions** | Files/folders (fuzzy over repo), other threads as context | M |
| 7 | **Slash menu** | Built-ins (/clear, /compact, /rename, /plan, /status, /export, /fork) merged with Claude's `commands` from `initialize` | M |
| 8 | **Queue / steer** | Queue follow-ups during a turn (edit, delete, send now); setting Queue vs Steer | S–M |
| 9 | **Effort, thinking, fast mode** | Picker + shift-tab cycling; persisted per thread | S |
| 10 | **Usage and context meter** | Tokens/cost per turn from `result`, context window bar, /status dialog, rate-limit banner from `rate_limit_event` | M |
| 11 | **Tool details** | Edit/MultiEdit as inline diffs, Bash stdout/stderr, Read previews, grouped consecutive tools | M |
| 12 | **Message actions** | Copy, edit and resend, collapse long messages, find in thread (⌘F) | M |
| 13 | **Composer polish** | Prompt history, draft per thread, large-paste chip, ⌘L focus, last-used model/options for new threads | S–M |

### Phase 2 — Threads, attention and app shell ✅

| Feature | Notes | Effort |
|---|---|---|
| Rename (manual + AI title) | Context menu, /rename, title generation via `claude -p` | S |
| Delete thread, offering to remove its worktree | Plus "delete worktree on archive" setting | S |
| Archived threads list | Restore / delete / delete all (Settings) | S |
| Pin threads and projects, Done/settled, unread | Sidebar groups: Pinned, Recent; read/unread state | M |
| Attention states | Working, needs approval, needs input, plan ready, failed, interrupted pills; jump to next unfinished chat after archive | S |
| Notifications | macOS notification + Dock badge when a turn finishes or needs input; activity toasts | S |
| Quit guard and quit-resume | Confirm with running chats; resume them after relaunch | M |
| Window state | Persist size/position, sidebar/panel widths, expanded projects | S |
| Context menus | Thread: rename, pin, mark unread, fork, handoff, copy path/ID, open in terminal, archive, delete. Project: reveal, copy path, rename, appearance, change folder, delete | M |
| Project appearance | Name, emoji or icon/colour | S |
| Chats without a project | Scratch workspace (⌥⌘N) | S |
| Onboarding | Provider check (installed / signed in), theme, first project | M |
| Single instance, crash recovery, auto-updater | Sparkle-style updates from GitHub releases; notarised signing | L |

### Phase 3 — Git, diff and review ✅

| Feature | Notes | Effort |
|---|---|---|
| Stage / unstage files, commit staged | Git panel with Staged/Changes lists | S |
| Generated commit message, PR title/body, branch name | Via a configurable "writing model" | S–M |
| Branches | List, create, checkout, stash-and-switch, conflict checks | M |
| Pull, publish branch, sync | Plus commit-and-push shortcut (⌃⌘P) | S |
| Create PR (`gh`) | Draft option; auto feature branch from uncommitted changes | M |
| PR badge and PR panel | Checks, reviews, comments, merge/close/reopen, "Fix review comments" prompt | L |
| Diff modes | Split view, word wrap, ignore whitespace, next/prev change (⌥↓/↑), file jump, truncation warning | M |
| Diff scopes | Working tree / staged / last turn / specific turn / compare with branch or commit | M |
| Checkpoints and revert | Hidden git ref per turn; "Revert to here"; per-turn diffs | L |
| Line comments to agent | Comment on diff lines, sent with the next prompt | M |
| Blame, file at revision | Blame popover in diff | S |
| Code review inbox | PRs and issues across projects' GitHub repos; Send to agent / Ask / Open | L |
| Managed worktrees list | Settings page with cleanup; retention policy | S |

### Phase 4 — Navigation, files and layout ✅

| Feature | Notes | Effort |
|---|---|---|
| Command palette (⌘K) | Threads (incl. message content), projects, actions, settings deep links | M |
| File search (⌘P), content search (⌘⇧F) | Index with `ignore` crate; ripgrep-style search | M |
| Explorer + file preview/editor | gpui-component ships an Editor with tree-sitter and LSP; Markdown/image/PDF preview, autosave, conflict handling | L |
| Open in external editor | VS Code, Cursor, Zed, JetBrains, Xcode, terminal apps; ⌘O favourite | S |
| Split chat (⌘\\) | Two threads side by side | M |
| Side chats (⌥⌘S) | Ephemeral fork in the right dock, other provider allowed, expires when idle | M |
| Right-dock pane system | Pluggable panes (Changes, Terminal, Files, PR, Browser…) instead of fixed tabs | M |
| Terminal extras | Splits (⌘D), search in scrollback, "Add selection to chat", full-width terminal mode, close confirmation | M |
| Keybindings | Settings recorder, conflicts, `~/.elyra/keybindings.json` with when-clauses; shortcuts sheet | M |
| Thread switching | ⌘1–9, ⌘⇧[ / ], Ctrl-Tab recent views, back/forward | S |
| Environment panel | Per-thread notes, pinned messages, recap, project instructions, local servers | M |
| Project spaces | Named groups of projects with a switcher | M |
| Appearance extras | System light/dark follow, density, chat width, theme editor/import, time format | M |

### Phase 5 — Providers ✅ (Codex postponed)

Prerequisite: a provider trait and a provider-neutral runtime event schema (Synara's
`providerRuntime.ts`: content deltas, items, requests, tasks, turn diff, token usage,
rate limits). Today `elyra-provider` emits Claude-shaped events.

| Provider | Protocol in Synara | Notable capabilities | Effort |
|---|---|---|---|
| Codex | `codex app-server` JSON-RPC | Steer, review, compact, fork, skills/plugins, live turn diff, images, multi-account, reset credits | L |
| Cursor | ACP (`cursor-agent acp`) | Model list, skills, fork, import | M (+ACP runtime L) |
| Devin, Factory Droid, Grok, Oh My Pi | ACP | Compact/fork/commands vary | S–M each on shared ACP runtime |
| OpenCode | SDK against `opencode serve` | Agents, commands, compact, fork, external server | M |
| Pi | In-process SDK (Node) | Steer, compact, skills | M (needs Node sidecar) |
| Antigravity (Gemini) | CLI per turn, stream-json | Skills, model list | M |

Also: Claude extras (steer active turn, /compact and compaction window, fork, stop
subagent task, list commands/agents/models, cache observations, artifacts toggle) · provider
settings (enable, order, binary path, env vars, sign-in status and "Sign in" terminal,
CLI version/update checks) · multiple accounts per provider · model discovery cache ·
starred model presets · provider handoff (continue the same task with another provider,
bounded recap) · fork (native or transcript reconstruction) · import threads/projects
from Claude Code and Codex history.

### Phase 6 — Orchestration and automation ✅

| Feature | Notes | Effort |
|---|---|---|
| Agent gateway (MCP) | Embedded MCP server injected into sessions: list/read/create threads, send message, wait, interrupt, set title/archive; lets agents fan out work | L |
| Automations | Schedules (once, interval, daily, weekdays, weekly, cron+tz), run modes, stop conditions, max iterations, failure policy, run history | L |
| Kanban (stable) / Tasks + Inbox *(beta)* | Draft / In progress / Done board; to-dos handed to an agent, status follows the chat | L |
| Thread goals | Persistent objective that auto-continues across turns, pause/resume | M |
| Debug mode | Reproduce-first interaction mode with badge | S |
| External MCP | Let Claude Desktop/Codex control Elyra with paired, scoped access and audit log | L |
| Export / recap / stats | Thread ZIP export, AI recap, profile stats and token heatmap | M |
| Hubs, Studio *(beta)* | Coordinator agent with worker queue, memory, git-versioned library | XL |

### Later / evaluate

| Feature | Why later | Effort |
|---|---|---|
| In-app browser, agent browser tools, WebMCP, login vault, cookie import | GPUI has no web view; needs an embedded WKWebView (wry) and CDP-style automation | XL |
| iOS Simulator pane and device tools | `simctl` plus frame streaming; macOS-specific | L |
| Computer use *(beta)* | Accessibility automation with safety shields | XL |
| AppSnap, voice dictation | Native capture helper; transcription service | M each |
| Headless server and remote web access | Synara's client/server split; Elyra is single-process by design | XL |
| Beta channel, telemetry | Release engineering | M |

## Architecture decisions to make early

1. **Provider runtime schema.** Introduce a provider trait and neutral event types before
   adding Codex, so the UI and store stop depending on Claude's wire format.
2. **Turn model in the store.** Synara persists turns, activities, pending approvals,
   plans and checkpoints separately. Elyra stores a flat item list. Turns as a first-class
   table make checkpoints, per-turn diffs, edit-and-resend, fork and usage accounting
   straightforward.
3. **Durability and recovery.** Synara reconciles runtime state on startup and can resume
   chats after quit. Decide how much of its event-sourcing to adopt; at minimum, durable
   queued turns and pending approvals.
4. **Embedded MCP server.** The agent gateway, external MCP and many agent tools (browser,
   devices) hang off it; one shared implementation serves all of them.
5. **Background work.** Automations and quit-resume need a scheduler that runs while
   windows are closed (menu-bar mode or a login item).

## Keyboard shortcut gaps

Already in Elyra: ⌘N, ⌘O, ⌘W, ⌘B, ⌥⌘B, ⌘J, ⇧⌘G, ⇧⌘T, ⌘, and terminal ⌘T, ⇧⌘W, ⇧⌘[ / ].

Missing from Synara's default map: ⌘K palette · ⌘P files · ⇧⌘F search · ⌘F find in thread ·
⌘1–9 jump to thread · ⇧⌘] / [ next/previous chat · ⌃Tab recent views · ⇧⌘N new thread in
latest project · ⌥⌘N chat without project · ⇧⌘T terminal-first thread · ⌥⌘C/X/R new
Claude/Codex/Cursor thread · ⌘\\ split chat · ⌥⌘S side chat · ⌘L focus composer ·
⇧⌘M model picker · ⌥] / ⌥[ cycle models · ⇧Tab cycle effort · ⇧⌘E effort picker ·
⇧⌘U usage · ⌥⌘U activity · ⇧⌘C copy thread ID · ⌘I import thread · ⌘D diff / terminal split ·
⇧⌘J full-width terminal · ⌥↓ / ⌥↑ next/previous change · ⌘S save · ⌃⌘P commit and push ·
⇧⌘B browser · ⌥⌘← / → and ⌥⌘1–9 spaces · project script shortcuts.

## Settings gaps

Synara has 16 settings sections. Elyra has Appearance (theme, UI font), Code (font) and
Terminal. Missing:

- **General:** default provider, default new-thread environment (local/worktree), delete
  worktree on archive, move sent messages to top, sidebar order and sections, welcome tour.
- **Appearance:** follow system light/dark, theme editor and share strings, app icon
  variants, density, chat width, font smoothing, time format.
- **Notifications:** activity toasts, desktop notifications.
- **Chat behaviour:** queue vs steer, streaming output, effort slider, PR/issue open target,
  diff wrapping, delete/archive/terminal-close confirmations.
- **Keybindings, Usage and limits, MCP connections, Agent providers, Models and writing,
  Agent skills, Managed worktrees, Archived threads, System tools (recovery, keybindings
  file, release history), Profile, AppSnap, Computer use.**

## Sources

Full per-feature inventories (with Synara file paths) were produced from:
`apps/marketing/content/docs/**`, `docs/*.md`, `KEYBINDINGS.md`, `BETA.md`, `REMOTE.md`,
`apps/web/src/{routes,components,lib,hooks}`, `apps/web/src/settingsSearchIndex.ts`,
`packages/contracts/src/{ws,rpc,orchestration,providerRuntime,git,automation}.ts`,
`apps/server/src/{orchestration,provider,git,automation,agentGateway,externalMcp}` and
`apps/desktop/src`.

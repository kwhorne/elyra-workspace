# Elyra Workspace

A focused, local-first desktop workspace for coding agents, written in Rust with
[GPUI](https://www.gpui.rs/) and [GPUI Kit](https://gpui-kit.com). Inspired by
[Synara](https://github.com/Emanuele-web04/synara): projects, threads, an agent
conversation, and the tools around it — changes, Git and a terminal — in one window.

| Layer        | Responsibility                                               |
| ------------ | ------------------------------------------------------------ |
| **Project**  | A local folder, preferably a Git repository                  |
| **Thread**   | One task: transcript, provider session, environment          |
| **Provider** | The locally installed coding agent: Claude Code or Elyra |
| **Tools**    | Changes/diff, commit/push, terminal                          |

## Documentation

The user guide is in [docs/](docs/README.md): getting started, threads, working
with agents, providers, Git and review, files, terminal, automations, the agent
gateway, settings, keyboard shortcuts and troubleshooting.

## Features

- Projects and threads in a sidebar, open threads as tabs, restored on restart
- Providers behind one `AgentSession` trait, chosen per thread: **Claude Code**
  (`claude -p` stream-json), **Elyra** and **Pi** (`--mode rpc`), and any
  [Agent Client Protocol](https://agentclientprotocol.com) agent — **Gemini CLI**,
  **Cursor Agent**, **OpenCode** built in, plus a custom ACP command
- Providers settings: enable/disable, executable, arguments, environment variables,
  named accounts per provider (e.g. separate `CLAUDE_CONFIG_DIR`s), installed version
  and "Sign in…" in Terminal; starred model presets in the model menu
- Fork a thread (native session branch where supported, otherwise with the
  conversation as context), continue a thread with another provider (handoff with
  recap), import Claude Code sessions (⌘I) that resume where they left off
- Agent chat: streaming text and reasoning, tool calls with live output, Edit/Write
  diffs, subagents with nested calls, task lists, plan review (approve / keep
  planning), structured questions (AskUserQuestion and Elyra extension dialogs),
  approvals (Allow / Allow for session / Deny), interrupt
- Composer: image/file attachments (paste, drop, picker), large-paste chips,
  `/` commands (built-ins + provider commands and skills), `@` file mentions,
  queued follow-ups with "send now" steering, prompt history (↑/↓), drafts per thread
- Model, effort and permission mode per thread; context-window meter and cost;
  find in thread (⌘F); copy, edit and resend messages
- Sidebar: pinned threads and projects, collapsible projects with emoji and colour,
  unread markers, done threads, archive with restore/delete, context menus,
  rename (manual or AI-generated title), chats without a project (⌥⌘N)
- Notifications and Dock badge when a background thread finishes or needs input;
  quit guard for running agents with resume after relaunch; window size and panel
  layout restored; first-run welcome; single instance per data directory; crash log;
  update check
- Local checkout or an isolated managed Git worktree per thread
- Git panel: branch switch/create (stash on conflict), fetch/pull/push/publish,
  staged and unstaged lists with stage/unstage/discard, diffs of the working tree,
  of a single agent turn or against any branch/commit, split or unified view,
  wrap, ignore whitespace, next/previous change (⌥↓/⌥↑), blame, line comments
  sent to the agent, generated commit messages, commit / commit & push (⌃⌘P)
- Checkpoints: the working tree is snapshotted before every turn (hidden refs,
  index untouched); restore files to before any message
- Pull requests (via `gh`): create with generated title/body and draft option,
  checks, reviews and inline comments, comment, merge/squash/rebase, ready,
  close/reopen, "ask the agent to address feedback"
- Code review inbox (⇧⌘R): PRs and issues across all projects' GitHub repos with
  filters, detail pane and "Send to agent"; managed worktrees dialog
- Terminal panel per thread (several terminals as tabs) built on alacritty_terminal,
  complete enough for full-screen TUIs (vim, htop, lazygit, Claude Code):
  mouse reporting (X10/normal/UTF-8/SGR, drag and any-motion), alternate screen with
  alternate scroll, selection (click-drag, double-click word, triple-click line,
  ⌥-drag block; Shift bypasses app mouse mode), copy/paste with bracketed paste,
  focus reporting, cursor shapes and blinking (DECSCUSR), wide/CJK/emoji cells,
  OSC 8 hyperlinks and URLs (⌘-click), OSC 52 copy, window title, IME and dead keys
- Command palette (⌘K) over threads (titles and message content), projects,
  commands and themes; file finder (⌘P) and content search (⇧⌘F), .gitignore-aware
- Files tab (⇧⌘E): explorer and a tree-sitter code editor with autosave, ⌘S,
  external-change detection (reload / overwrite), Markdown and image preview,
  "mention in chat"; open the project or file in an external editor (⌘O)
- Thread navigation: ⌘1–9, ⇧⌘[ / ⇧⌘], ⌃Tab most recent, ⌘[ / ⌘] back/forward;
  split chat (⌘\\) shows two threads side by side
- Side chats (⌥⌘S): a branch of the current agent session in the tools panel, to
  ask without derailing the thread; promote to a thread or discard; unused ones expire
- Context tab (⇧⌘I): thread notes, pinned messages, AI recap, project instructions
  (appended to every agent's system prompt) and dev servers running in the project
- Terminal extras: split (⌘D), scrollback search (⌘F), add selection to chat,
  full-width terminal (⇧⌘J), confirmation before closing a busy terminal
- Project spaces to group projects and filter the sidebar
- Agent gateway (Settings → Agents & MCP): agents get MCP tools to list, read,
  create, message, wait for, interrupt, rename and archive threads, so one agent can
  fan out work. Pair Claude Desktop, Codex or Claude Code as external clients
  (read-only or full access, copy-paste config, `elyra mcp-bridge` for stdio); every
  call is in the audit log
- Automations (⌥⌘A): prompts on a schedule (once, every N minutes, daily, weekdays,
  weekly, cron with time zone), each run in a new or the same thread, stop phrase,
  max runs, failure policy (pause, keep going, retry once) and run history
- Task board (⌥⌘T): Draft / In progress / Done with drag and drop; "Start" hands a
  card to an agent and the card follows the thread
- Thread goals (Context tab) that keep the agent going turn after turn within a
  budget, debug mode (reproduce first), thread export as ZIP, usage statistics
- Keyboard shortcuts sheet (⌘/); override any shortcut in `~/.elyra/keybindings.json`
- Settings window (⌘,): themes (Default Dark/Light, Tokyo Night, Palenight, Dracula,
  Nord, plus custom themes from `~/.elyra/themes`; terminal colors follow the theme),
  follow system light/dark, conversation width and density, external editor,
  interface/code/terminal fonts and sizes, terminal line height, cursor,
  Option-as-Meta, copy on select, shell, scrollback
- Everything persisted in SQLite (`~/.elyra/state.db`)

## Requirements

- Rust 1.85+ (edition 2024)
- macOS (primary target; GPUI also supports Linux and Windows)
- At least one provider: [Claude Code](https://docs.claude.com/en/docs/claude-code)
  (`claude`, logged in) and/or [Elyra](https://www.npmjs.com/package/@elyracode/coding-agent)
  (`npm install -g @elyracode/coding-agent`)
- `git`

## Run

```console
cargo run --release
```

### macOS app bundle

```console
scripts/bundle-macos.sh        # -> target/release/bundle/Elyra Workspace.app
```

Signs ad hoc; set `CODESIGN_IDENTITY="Developer ID Application: …"` to sign for
distribution. Bundle id: `com.gets.elyra-workspace`.

### App icon

The icon is Elyra Conductor's Lyra constellation recolored to Elyra yellow
(`#fbd22d`). Regenerate `assets/icon/` from the Conductor icon with
`python3 scripts/recolor-icon.py [path/to/icon.icns]` (needs Pillow).

Use `ELYRA_HOME=/some/dir` for an isolated data directory (development, tests) and
`ELYRA_NO_ACTIVATE=1` to open the window in the background.

Provider smoke test (real turn, prints neutral events):
`cargo run -p elyra-provider --example smoke -- elyra <dir> "<prompt>"`

Any ACP agent: `ACP_COMMAND="npx -y @agentclientprotocol/claude-agent-acp" cargo run -p elyra-provider --example smoke -- acp <dir> "<prompt>"`

## Keyboard

| Shortcut | Action              |
| -------- | ------------------- |
| ⌘K       | Command palette     |
| ⌘P       | Find file           |
| ⇧⌘F      | Search in files     |
| ⌘N       | New thread          |
| ⇧⌘O      | Add project         |
| ⌘O       | Open in external editor |
| ⌘1–9     | Go to tab           |
| ⌃Tab     | Most recent thread  |
| ⌘[ / ⌘]  | Back / forward      |
| ⌘\       | Split chat          |
| ⌥⌘S      | New side chat       |
| ⇧⌘K      | Fork thread         |
| ⌘I       | Import from Claude Code |
| ⇧⌘E      | Files               |
| ⇧⌘I      | Context and notes   |
| ⇧⌘J      | Full-width terminal |
| ⌘/       | All shortcuts       |
| ⌘W       | Close tab           |
| ⌘B       | Toggle sidebar      |
| ⌥⌘B      | Toggle tools panel  |
| ⌘J       | Toggle terminal     |
| ⇧⌘G      | Show changes        |
| ⇧⌘T      | Toggle theme        |
| ⌃C       | Interrupt (composer)|
| ⌘F       | Find in thread      |
| ⌘L       | Focus composer      |
| ⌥⌘N      | New chat (no project) |
| ⌘,       | Settings            |

Terminal: ⌘T new terminal, ⌘D split, ⌘F search scrollback, ⇧⌘W close terminal,
⇧⌘[ / ⇧⌘] switch, ⌘C/⌘V copy/paste,
⌘K clear, ⌘A select all, ⌘←/⌘→ line start/end, ⌘⌫ delete line, ⌥←/⌥→ word,
⇧PgUp/⇧PgDn or ⌘↑/⌘↓ scroll, ⌘-click opens links.

Option composes characters by default (needed for `[ ] { } | @ ~` on Nordic layouts);
enable *Use Option as Meta key* in Settings → Terminal for Emacs-style Meta.

## Architecture

```text
crates/
├── elyra-core      domain model, SQLite store + migrations, data paths, login-shell PATH
├── elyra-provider  AgentSession trait, neutral events, Claude Code + Elyra adapters
├── elyra-git       git CLI wrapper: status, diff parsing, commit, push, worktrees
├── elyra-terminal  PTY + VT emulation (alacritty_terminal): snapshots, keys, mouse, selection
└── elyra-app       GPUI application (binary `elyra`)
```

`elyra-app` modules: `app_state` (store, project/thread lists, live sessions),
`thread_session` (one thread's provider process and transcript), `workspace`
(window layout), `thread_view` + `transcript` (conversation), `changes_view`,
`terminal_view` (terminal rendering/input + panel), `preferences` + `themes` +
`settings_window` (Settings).

## Development

```console
cargo test --workspace
cargo clippy --workspace
cargo fmt --all
```

## License

MIT

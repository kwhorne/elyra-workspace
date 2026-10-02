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

## Features (MVP)

- Projects and threads in a sidebar, open threads as tabs, restored on restart
- Providers behind one `AgentSession` trait: **Claude Code** (`claude -p` stream-json)
  and **Elyra** (`elyra --mode rpc`), chosen per thread
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
- Changes panel: changed files, unified diff, discard, commit, push
- Terminal panel per thread (several terminals as tabs) built on alacritty_terminal,
  complete enough for full-screen TUIs (vim, htop, lazygit, Claude Code):
  mouse reporting (X10/normal/UTF-8/SGR, drag and any-motion), alternate screen with
  alternate scroll, selection (click-drag, double-click word, triple-click line,
  ⌥-drag block; Shift bypasses app mouse mode), copy/paste with bracketed paste,
  focus reporting, cursor shapes and blinking (DECSCUSR), wide/CJK/emoji cells,
  OSC 8 hyperlinks and URLs (⌘-click), OSC 52 copy, window title, IME and dead keys
- Settings window (⌘,): themes (Default Dark/Light, Tokyo Night, Palenight, Dracula,
  Nord — terminal colors follow the theme), interface/code/terminal fonts and sizes,
  terminal line height, cursor, Option-as-Meta, copy on select, shell, scrollback
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

## Keyboard

| Shortcut | Action              |
| -------- | ------------------- |
| ⌘N       | New thread          |
| ⌘O       | Add project         |
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

Terminal: ⌘T new terminal, ⇧⌘W close terminal, ⇧⌘[ / ⇧⌘] switch, ⌘C/⌘V copy/paste,
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

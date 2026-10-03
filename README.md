# Elyra Workspace

A local-first desktop workspace for coding agents. Add your project folders, give
each task its own thread, and work with Claude Code, Codex or another agent, with
the changes it makes, Git, a terminal and your files in the same window. Written in
Rust with [GPUI](https://www.gpui.rs/) and [GPUI Kit](https://gpui-kit.com), inspired by
[Synara](https://github.com/Emanuele-web04/synara).

**[Download the latest release](https://github.com/kwhorne/elyra-workspace/releases/latest)**
— a signed and notarized DMG for Macs with Apple Silicon (macOS 12 or later). The app
updates itself after that.

## Highlights

- **Threads for every task**: tabs, split view, side chats, fork, and handoff of a
  task to another agent. Threads can work in the project folder or in their own Git
  worktree.
- **The agent's work, visible**: streaming replies, tool calls with live output and
  diffs, task lists, plans, questions and approvals, plus a checkpoint before every
  turn that you can restore.
- **Git and review**: staging, generated commit messages, per-turn diffs, blame,
  line comments to the agent, pull requests and a code review inbox (via `gh`).
- **Files and terminal**: a command palette, file and content search, an editor,
  and a full terminal for vim, htop or lazygit, with splits and search.
- **Automation**: scheduled prompts, a task board, and thread goals that keep an
  agent going until the job is done.
- **Agent gateway (MCP)**: let agents start and steer other threads, or connect
  Claude Desktop and Codex. Every call is in an audit log.
- **Local and private**: no account and no telemetry. Data lives in `~/.elyra`, and
  the agents run as the command-line tools you install.

## Supported agents

| Agent | How it connects |
| --- | --- |
| [Claude Code](https://docs.claude.com/en/docs/claude-code) | `claude` (stream-json) |
| [Codex](https://github.com/openai/codex) | `codex app-server` |
| [Elyra](https://www.npmjs.com/package/@elyracode/coding-agent) and Pi | `--mode rpc` |
| Gemini CLI, Cursor Agent, OpenCode | [Agent Client Protocol](https://agentclientprotocol.com) |
| Any other ACP agent | a custom command |

Install and sign in to at least one of them. Elyra Workspace uses their own logins
and needs no API keys.

## Documentation

- [User guide](docs/README.md): getting started, threads, working with agents,
  providers, Git and review, files, terminal, automations, the agent gateway,
  settings, keyboard shortcuts and troubleshooting
- [Development](docs/development.md): building from source, architecture, testing
  and releasing
- [Changelog](CHANGELOG.md)

## Building from source

Requires Rust 1.85 or later and `git`, on a Mac with Apple Silicon.

```console
cargo run --release
```

See [Development](docs/development.md) for the app bundle, tests and the release
process.

## License

MIT — see [LICENSE](LICENSE).

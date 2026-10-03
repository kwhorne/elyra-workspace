# Changelog

## 0.1.0

First release, for macOS (Apple Silicon). See [docs/](docs/README.md) for the user
guide.

- Projects and threads with tabs, pinning, spaces, archive, fork, side chats,
  split chat, handoff to another provider, import from Claude Code and ZIP export
- Agents: Claude Code, Elyra, Pi and Agent Client Protocol agents (Gemini CLI,
  Cursor Agent, OpenCode, or any ACP command), with accounts per provider
- Conversation: streaming, tool calls with diffs, subagents, task lists, plans,
  questions, approvals, attachments, `@` mentions, `/` commands, queue and steer,
  checkpoints with restore, find in thread, debug mode
- Git: staging, commits with generated messages, branches, sync, per-turn diffs,
  blame, line comments to the agent, worktrees per thread, pull requests and a
  code review inbox (via `gh`)
- Files tab with editor, command palette, file finder and content search,
  external editors
- Terminal with full TUI support, splits, scrollback search and "add to chat"
- Context tab: notes, pinned messages, recap, project instructions, local servers
  and thread goals
- Automations on schedules, a task board, usage statistics
- Agent gateway (MCP) for agents and external clients, with an audit log
- Settings: themes (incl. custom), fonts, terminal, providers, shortcuts

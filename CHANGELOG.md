# Changelog

## 0.1.5

- **Help → Elyra Workspace Documentation** (and "Documentation" in the command
  palette) opens the user guide on
  [elyracode.com/docs/workspace](https://elyracode.com/docs/workspace).
- Update checks keep working when GitHub's API limit is reached (60 requests an
  hour per network, shared by everyone on it): the app then reads the latest
  version from the release page instead.

## 0.1.4

No changes to the app itself.

- Releases are now built, signed, notarized and published by GitHub Actions
  when a version tag is pushed; this is the first release made that way.
- New README with a screenshot, and a development guide
  ([elyracode.com/docs/workspace/development](https://elyracode.com/docs/workspace/development))
  for building and releasing.

## 0.1.3

- Maintenance release with no functional changes, published to verify that
  0.1.2 updates itself automatically.

## 0.1.2

- **Automatic updates.** New versions are downloaded and verified in the
  background (checksum, same Developer ID team, notarization, version) and
  installed when you restart or quit; the app relaunches itself. Checks run at
  launch and every six hours. Settings → General can turn off automatic
  downloads. Apps running from the disk image or a read-only folder still get a
  download link.

## 0.1.1

- **Codex** as a provider through `codex app-server`: streaming, reasoning,
  commands with live output, diffs, plans, questions, approvals mapped to Codex's
  approval policy and sandbox, steer, compact, models, effort, native fork and
  resume, project instructions, and the agent gateway (MCP)
- Import Codex threads alongside Claude Code sessions (⌘I)
- Fix: the shortcuts and command palette entries for fork (⇧⌘K), import (⌘I),
  the task board (⌥⌘T), automations (⌥⌘A), statistics and export were missing

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

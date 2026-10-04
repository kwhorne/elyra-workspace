# Changelog

## 0.3.1

- **Conventional Commits.** Generated commit messages and pull request titles
  follow `type(scope): summary`, such as
  `fix(nightwatch-exceptions): employee service create`, and *Commit* and
  *Create pull request* check the format and say what to fix. On by default;
  turn it off in Settings → Code → Commits and pull requests.

## 0.3.0

- **Best of N.** Give one task to several agents at once (command palette →
  *Best of N…*), each in its own worktree. Compare their changes, cost and
  replies side by side, and bring the one you pick into the project, staged for
  review. See [Projects and threads](docs/projects-and-threads.md#best-of-n).
- **Second opinion.** *Second opinion from ▸* in the thread menu has another
  provider review the last turn's diff in a read-only side chat; one click hands
  its findings back to the original agent.
- **Errors from the browser, one click away.** New console errors, uncaught
  exceptions and failed requests on a local page show up as a chip above the
  message box; *Add to message* attaches them.
- **Pictures before and after a turn.** With a local page in the Browser tab,
  each turn gets a picture of the page from before and after it in the
  conversation.
- **Budgets.** Set a spending limit for a thread in the Context tab. At 80% Elyra
  says so; at the limit goals pause and ask you.
- The browser now records uncaught exceptions and unhandled promise rejections
  for agents too, with their messages.

## 0.2.3

- Elyra threads have the gateway's `mcp__elyra__…` tools from their first
  message (with Elyra 0.9.47 or later). The gateway token now reaches Elyra in
  its own environment variable, so Elyra's cached tool list survives the new
  token each session.
- The command palette (⌘K) scrolls to keep the selected row in view when you
  move with the arrow keys.

## 0.2.2

- Elyra threads get the agent gateway too (Elyra 0.9.46 or later), as
  `mcp__elyra__…` tools. Pi still doesn't.

## 0.2.1

First release with the browser (0.2.0 was tagged but never published).

- **Browser.** Each thread has a web browser in the tools panel (**⇧⌘B**, or
  click a local server in the Context tab) for looking at the app you're
  building. See [Browser](docs/browser.md).
- With the agent gateway on, agents can open their thread's local dev server in
  it and inspect the page: DOM outline, elements and computed styles, console,
  network calls and a screenshot. Only pages served from this Mac (localhost,
  `*.test`, `*.local`) can be opened or read.
- The browser and its agent tools are adapted from Litr by Wirelabs AS, used
  under the MIT licence with permission.

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

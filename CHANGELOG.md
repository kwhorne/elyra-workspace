# Changelog

## 0.7.0

- **Félagi tasks and time.** Connect [Elyra Félagi](https://github.com/kwhorne/Felagi)
  in Settings → Félagi (the token is kept in the macOS Keychain), and the task
  board gets a Félagi tab with the issues assigned to you. **Start** opens a
  thread on an issue, moves it to In progress and starts Félagi's timer; the
  thread's banner shows the issue and the timer, and **Report…** writes back
  what was done (drafted by the agent), the status and the hours.

## 0.6.0

More with [Elyra Grove](https://github.com/kwhorne/grove):

- **Worktrees with their own running app.** In projects Grove runs as an app, a
  thread's new worktree comes from Grove (`grove try --new`): its own copy of
  the database, migrated, and its own `.test` address, which opens in the
  thread's browser. Best of N candidates each run as their own app. Removing the
  worktree takes the site and database copy with it.
- **The database in every checkpoint.** In Grove apps the checkpoint before each
  turn also snapshots the database (SQLite, MySQL, PostgreSQL or ElyraSQL), and
  *Restore files* puts it back, undoing the agent's data changes and migrations
  with its code. The latest 10 per thread are kept.
- **Did the fix work?** Server errors you send to the agent are replayed when
  its turn ends (`grove replay --same-data`), and the conversation says *was
  500, now 200 ✓* or *still 500*.
- The error chip, the Grove box in the Context tab and the browser's start page
  follow a worktree's own site.

## 0.5.0

Working with [Elyra Grove](https://github.com/kwhorne/grove), the local
development environment:

- **Server errors in the error chip.** Requests Grove records with a 5xx status
  for the project's app show up next to browser errors; *Add to message*
  attaches Grove's explanation: the request, its SQL and mail, and the stack
  trace from the error log.
- **Grove's tools for agents.** In projects Grove runs as an app, agents get
  Grove's MCP server (read-only) next to the agent gateway, so they can look up
  recent requests, logs and the database schema themselves. Turn it off in
  Settings → Agents & MCP.
- **Grove in the Context tab.** The app's `.test` address, its `grove dev`
  processes with Start and Stop, and caught mail; the Browser tab's start page
  offers the address too.
- MCP servers given to agents can now be local commands as well as HTTP servers.

Also new:

- **Material theme**, after [Material Theme for VS Code](https://marketplace.visualstudio.com/items?itemName=vihuvac.material-theme-for-vscode):
  blue-grey surfaces, the Material colours and a teal accent. Settings →
  Appearance.

## 0.4.0

- **Point at an element.** The target button in the Browser tab lets you click
  something on the page; your message gets a picture of it and its selector,
  position, text, computed styles and HTML, so the agent knows exactly which
  element you mean.
- **Agents ask before changing another thread.** The first time an agent sends a
  message to, stops, renames or archives another thread through the gateway, you
  choose Don't allow, Allow once or Always allow; remembered pairs can be
  forgotten in Settings → Agents & MCP. Threads an agent starts itself are its
  own and need no asking.
- **Updates wait for your agents.** Restarting for an update while agents are
  working offers *Restart when they finish*, which restarts by itself once no
  turn is running, as well as *Restart now*.

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

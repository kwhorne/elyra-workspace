# Context, notes and goals

The **Context** tab in the tools panel (**⇧⌘I**) collects everything around the
active thread.

## In the context

The top of the tab shows what fills the agent's context window. It counts the
conversation since the last **Clear**, next to what the agent last reported:

- A bar and one row per part: the **agent setup** (its system prompt, tool
  definitions and instructions, i.e. whatever the conversation doesn't account
  for), **your messages**, **agent replies**, **tool calls** and **tool results**.
  Each row has its size, its share of the context and roughly what carrying it
  cost in the last turn.
- **Largest**: the biggest single entries, such as a long log a command printed.
  Click one to scroll the conversation to it.
- When the agent has compacted earlier parts, a line says roughly how much.
- When one tool result takes a large part of the context, a hint says so. The
  agent reads the whole context again on every call it makes, so a large result
  costs on every step until the conversation is compacted.

**Compact** has the agent summarise the conversation so far. **Clear** starts a
new conversation in the thread; the transcript stays. Neither is available while
a turn runs.

The numbers are estimates (about four characters a token). Prices are each part's
share of what the last turn cost. Agents that don't report their context use
(some ACP agents) get the conversation estimate only. Click the context meter
below the message box to open this tab.

## Goal

A goal keeps the agent working, turn after turn, until a larger objective is
reached. For example: *All tests pass and the checkout flow handles discounts*.

1. Write the goal and press **Start goal**. The agent gets the goal as its next
   message, with instructions to take one concrete step at a time and verify it.
2. After each turn, Elyra sends it on automatically, until either:
   - the agent ends a reply with **GOAL ACHIEVED**: the goal is *Achieved*,
   - the agent ends a reply with **GOAL BLOCKED** because it needs you, or a turn
     fails: the goal is *Paused*, or
   - the turn budget runs out (10 automatic turns by default, set in
     **Settings → Agents & MCP**): the goal stops.

   You get a notification in each case.
3. **Pause** stops after the current turn. **Resume** continues with a fresh
   budget. **Clear** removes the goal.

Messages you queue yourself are sent before the goal continues. While a goal is
active, permission requests still stop and wait for you, so choose the permission
mode with that in mind.

## Budget

A spending limit for the thread, in US dollars. The **Budget** section shows what
the thread has cost so far, with a bar when a limit is set. Type an amount and
press **Set** (or Enter); **Remove** takes the limit away.

- At 80% of the limit, Elyra notes it in the conversation.
- At the limit, a running goal pauses and asks you, and a goal can't be started
  or resumed until you raise the limit. Messages you send yourself still go.

Costs are the ones the agent reports with each turn. Claude Code reports them;
agents that don't report costs count as free, so a limit has no effect on them.

## Notes

A notepad for the thread, saved as you type. The agent doesn't see it.

## Recap

**Generate** asks the agent for a short summary of the conversation: the goal,
what's done, decisions made and what's left. **Refresh** writes a new one. The
recap is saved with the thread and used when you hand the thread off to another
provider.

## Pinned messages

Hover a message in the conversation and press the **pin**. Pinned messages are
listed here so important answers are easy to find. Unpin them here or in the
conversation.

## Project instructions

Instructions that every agent in the project gets in addition to its normal
system prompt, such as conventions, commands to run, or things to avoid. They
apply from the next message, in every thread of the project. Claude Code, Codex,
Elyra and Pi support this. ACP agents don't take extra instructions.

### Rules agents learn

When you correct an agent (*"no, we always validate with Form Requests"*), it can
propose that as a rule: a card in the thread with the rule and what taught it.
**Add to AGENTS.md** puts it under *Learned rules* in the repository's AGENTS.md
(or CLAUDE.md or GEMINI.md, when that is what the project has), so every agent and your team
get it; commit it with your changes. **Add to project instructions** keeps it in
Elyra only. **Dismiss** drops it.

## Checks after each turn

*Done means green.* Give the project a check command (tests, lint, or both,
such as `cargo test` or `vendor/bin/pint --test && php artisan test`). Elyra
suggests one from the project's files; **Use …** fills it in, and **Run now**
tries it.

- After every turn that changed files (Elyra compares the Git working tree
  before and after), the command runs in the thread's folder, with your login
  shell and `CI=1`. A turn that changed nothing isn't checked.
- The thread stays busy while the checks run. **Checks passed** or **Checks
  failed** appears in the conversation; click it for the output.
- When they fail, the end of the output goes back to the agent to fix
  (*Sent the failures to the agent*), and the checks run again. After **Settings
  → Agents & MCP → Automatic fixes when checks fail** (2 by default) the thread
  stops, marked failed, and you are told. 0 only reports the failure. No
  automatic fix is sent when you have queued a message or the budget is used up.
- **A stronger fix**: when a first automatic fix didn't make them pass, the next
  one gets the agent's **Escalation model** (Settings → Providers, e.g. `opus`)
  and its highest effort; the thread goes back to its own model afterwards. With
  no escalation model set, only the effort goes up.
- **Stop** stops the checks too. A check is stopped after 20 minutes.
- Notifications, Félagi reports and `wait_for_thread` wait for the checks, so
  *finished* means the checks passed.

The command applies to every thread in the project. Empty turns it off.

### Who broke it?

When the checks fail, the problem may come from earlier than the last turn:
another thread, another agent, or a turn hours ago. **Who broke it?** on a
**Checks failed** card finds the instruction that made them fail:

1. Every turn has a snapshot of the folder taken before it, also turns in other
   threads working in the same folder. With the current state, that is a line of
   states.
2. The check runs against them in a temporary copy next to the project: first
   the current state and the oldest (up to 40 turns back), then halving the
   range until it finds the turn after which the checks fail. Thirty turns take
   about seven runs.
3. A card names the message behind that turn, its thread, agent and time, and
   the end of the failing output. **Send the failure there** asks that thread to
   fix it; **Open thread** shows it.

Your working folder isn't touched. The temporary copy shares what Git ignores at
the top of the project (dependencies such as `vendor` and `node_modules`, and
`.env`) by link, so the checks can run there; it's removed afterwards. **Stop**
ends the search. If the checks pass in the temporary copy, the failure depends on
something outside the project's files (a database, a service); if they already
failed before the oldest turn, none of the turns looked at caused it. Turns that
ran at the same time in one folder can't be told apart.

Agents can ask too, with the gateway's `who_broke_it` tool.

### How the agents do on your code

**Usage statistics** (command palette) has a table of how each agent and model
does on your projects, counted per instruction: a message from you, with the
automatic fixes after it.

- **Green first try**: the checks passed after the agent's first answer.
- **Green in the end**: they passed after the automatic fixes.
- **Fixes each** and **Cost each**: automatic fixes and reported cost per
  instruction.

Agents with fewer than three instructions that were checked are greyed out.
Side chats aren't counted. A new thread shows a line with the same numbers for
its project when at least two agents have enough to compare, for example
*Checks green on the first try in shop: Codex 9 of 10 ($0.12 each), Claude Code
7 of 10 ($0.40 each)*.

## Shared with every agent

*One setup for every agent.* Claude Code reads a project's `.mcp.json` and its
skills itself; Elyra gives them to the other agents (Codex, Elyra, Pi and ACP
agents) too, so switching agents keeps the same tools.

- **MCP servers in `.mcp.json`** (Claude Code's format: `command`, `args`, `env`,
  or `url` and `headers`; `${VAR}` and `${VAR:-default}` are filled in from the
  environment). The file comes with the repository and its commands run on your
  Mac, so they're only passed on after **Allow for every agent**. If the file
  changes, allow it again. **Stop sharing** takes it back. Agents get the change
  from their next message.
- **Skills** in `.claude/skills/<name>/SKILL.md`, `.agents/skills/…` and
  `.gemini/skills/…`, in the project and in your home folder. Each agent gets a
  list (name, description, path) of the skills it doesn't find itself, and reads
  a skill's SKILL.md when a task matches it. Claude Code finds `.claude/skills`;
  Gemini CLI finds `.gemini/skills` and `.agents/skills`; the other agents get
  them all.
- **AGENTS.md for Gemini CLI.** Gemini CLI reads only `GEMINI.md` by default. In
  a project without one, Gemini threads get the project's `AGENTS.md` (or
  `CLAUDE.md`) along with their instructions, unless your Gemini settings
  (`context.fileName`) already name that file.

## Local servers

Development servers running from the thread's folder, for example `npm run dev`
in the terminal or a server the agent started. Each one is listed with its port
and process. Click the address to open it in the thread's [browser](browser.md),
or press the refresh button to look again.

### Grove

When [Elyra Grove](https://github.com/kwhorne/grove) runs the project as an app
(Laravel, PHP or a proxied dev server, not a folder it only parks), a **Grove**
box shows:

- its address, such as `https://shop.test`, which opens in the thread's browser
  (the browser's start page offers it too)
- whether `grove dev` runs the app's dev processes (Vite, queue worker…), with
  **Start** / **Stop**
- how many mails Grove has caught, when there are any

Elyra asks the `grove` command line, from your `PATH` or `~/.grove/bin`. Without
Grove nothing changes.

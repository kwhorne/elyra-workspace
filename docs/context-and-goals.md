# Context, notes and goals

The **Context** tab in the tools panel (**⇧⌘I**) collects everything around the
active thread.

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

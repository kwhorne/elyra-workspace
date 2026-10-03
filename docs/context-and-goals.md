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
and process. Click the address to open it in the browser, or press the refresh
button to look again.

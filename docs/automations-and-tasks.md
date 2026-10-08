# Automations and tasks

## Automations

An automation sends a prompt to an agent on a schedule: nightly test runs,
dependency updates, a weekly summary of open issues. Open **Automations** with
**⌥⌘A** or the calendar button at the top of the sidebar.

> Automations run while Elyra Workspace is open. A run that was due while the app
> was closed starts shortly after you open it again.

### Creating one

**An agent can propose one.** Ask in a thread, for example *"check our
dependencies every weekday night"*: with the [agent gateway](agent-gateway.md)
on, the agent proposes an automation, and a card in the thread shows its name,
schedule and next run, the agent, project and permission mode, and the prompt.
**Create** schedules it as it is, **Edit…** opens it in the editor first (saving
creates it), **Dismiss** drops it. Nothing runs unless you accept.

By hand:

Press **New** and fill in:

| Field | Meaning |
| --- | --- |
| Name | Shown in the list, and as the title of each run's thread |
| Prompt | What the agent should do each time |
| Project | Where it runs |
| Agent | Which provider runs it |
| Permissions | The permission mode for its threads. Nobody may be around to approve, so *Accept edits* or *Full access* is usual. |
| Schedule | See below |
| Each run | **New thread** for every run, or **Same thread** to continue one conversation |
| If a run fails | **Pause** the automation, **Keep going**, or **Retry once** right away |
| Stop when the reply contains | Optional. When the agent's reply contains this text, the automation switches itself off. Example: ask the agent to reply *ALL GREEN* when done, and stop on that. |
| Max runs | Optional. Switch off after this many runs. |

### Schedules

| Schedule | Example |
| --- | --- |
| Daily | every day at 09:00 |
| Weekdays | Monday to Friday at 07:30 |
| Weekly | Fridays at 16:00 |
| Every N minutes | every 30 minutes |
| Once | 2026-12-24 08:00 |
| Cron | `0 9 * * 1-5`: standard five fields (minute, hour, day of month, month, day of week; 0 or 7 = Sunday) |

Times are in your Mac's time zone, which is saved with the automation, so
daylight-saving changes are handled.

### Running and history

- The switch on each row turns an automation on or off. Turning it on schedules
  the next run from now.
- **Run now** runs it immediately without changing the schedule.
- Select an automation to see its prompt, its rules and its **run history**: when
  each run started, how long it took, and its result:
  - *Succeeded*
  - *Failed*
  - *Stopped*: the stop phrase appeared
  - *Skipped*: the previous run or the thread was still busy
- **Open thread** opens the thread where a run happened. If a run waits for an
  approval, its history entry says so.

## Task board

The task board (**⌥⌘T**, or the board button in the sidebar) is a simple Kanban
board with three columns: **Draft**, **In progress** and **Done**.

- Type in **Add a task…** at the top of *Draft* and press Enter. The task belongs
  to the project selected in the top-right menu, or the first project when *All
  projects* is selected.
- **Start** hands a task to an agent. Elyra creates a thread in the task's
  project, named after the task, and sends the title and notes as the first
  message. The card moves to *In progress*.
- Dragging a draft card to *In progress* starts it the same way. You can drag any
  card to any column.
- Once a card has a thread, it shows the thread's state: *Working*, *Needs you*,
  *Replied*, *Failed* and so on. **Open** jumps to the thread. When you mark the
  thread as done or archive it, the card moves to *Done* automatically.
- The **⋯** menu on a card: **Edit…** (title and notes), **Move to…**, **Delete**.
- The menu in the top-right corner filters the board by project.

## Félagi tasks and time

With [Elyra Félagi](https://github.com/kwhorne/Felagi) connected, the task board
has a **Félagi** tab next to **Local**: the issues assigned to you, in the
columns *To do*, *In progress*, *In review* and *Done*, each with its identifier
(such as `ACM-231`), priority, and time spent against its estimate.

### Connecting

In Félagi, make a personal token under **Settings → API tokens**, with **Read and
write** so work can be reported back. In Elyra Workspace, open **Settings →
Félagi → Connect…**, give Félagi's address and the token, and press
**Connect**. Elyra checks the token with Félagi and keeps it in the macOS
Keychain, not in its own database. **Disconnect** removes it again.

### Choosing what to see

With a project selected in the board's filter, the menu at the top of the Félagi
tab links it to a Félagi project, so the tab shows only that project's issues.
*All my issues* shows everything assigned to you. The tab asks Félagi again every
90 seconds; the refresh button asks right away.

### Working on an issue

**Start** on a card opens a thread in the selected project, named after the
issue, and sends the issue's text and acceptance criteria as the first message.
The issue moves to *In progress* and Félagi's timer starts on it. **Open thread**
jumps to it later; **Open in Félagi** opens the issue in your browser.

A thread working on an issue shows a banner above the message box: the issue,
its status in Félagi and, while it runs, the timer.

### Reporting back

**Report…** on the banner writes back to Félagi:

- **What was done**: a comment. The thread's agent writes a draft from the
  conversation and the uncommitted changes; edit it as you like.
- **Status**: suggested as *In review*; any status can be chosen.
- **Time to log**: filled in from Félagi's timer, in Félagi's notation
  (`1h 30m`, `45m`, `1.5h`). Leave it empty to log nothing.
- **Pictures**: when the thread has [pictures of the page](browser.md#pictures-before-and-after-a-turn),
  the latest before and after are attached to the comment, so whoever reads the
  issue sees what changed. Untick it to leave them out.

When the branch has a pull request, the draft ends with a link to it. Pull
requests made from a thread on an issue use the issue as their
[scope](git-and-review.md#conventional-commits), such as
`fix(acm-231): login loop on safari`, and end their description with
`Félagi: ACM-231`.

**Send to Félagi** logs the hours (replacing the running timer, so nothing is
counted twice), posts the comment and sets the status, and the thread notes what
was sent. A read-only token can show issues but not report.

### Filing a server error

When [Elyra Grove](context-and-goals.md#grove) reports a server error in the
error chip, **File in Félagi** opens an issue for the newest one: type
*exception*, high priority, in the Félagi project linked to this one, with Grove's
explanation (the request, its SQL and mail, the stack trace) as its description.
A thread that isn't working on an issue yet is linked to the new one, so you can
fix it there and report back.

### Running Félagi's agents on this Mac

Félagi's agents (Freya, Bragi …) normally run through Félagi's own daemon. Elyra
Workspace can be that runtime instead: work Félagi gives an agent then runs as a
thread here, where you can watch it, answer its questions and step in.

1. In Félagi, **Admin → Runtimes → Connect a machine** gives a daemon token
   (`fdt_…`).
2. In Elyra Workspace, **Settings → Félagi → Run Félagi's agents on this Mac**:
   set the **daemon token** (kept in the Keychain), give the Mac a unique
   **machine name** (such as `kh-macbook`; blank uses the computer's name), and
   turn on **Run agents here**.
3. The Mac appears among Félagi's runtimes, once per agent kind it can run
   (Claude Code, Codex, Elyra). In Félagi, choose it as the runtime of the agents
   that should work here.

When Félagi hands such an agent a run, Elyra Workspace:

- finds the project whose Git remote is one of the repositories the run names
  (refusing the run if none is open here), and starts a thread there in its own
  worktree (with its own [Grove](git-and-review.md#worktrees-with-their-own-running-app)
  site when Grove runs the app), named after the agent and the issue
- gives the agent its instructions, skills, the workspace's context and the issue
- sends what the agent does to Félagi as it happens (its replies, tool calls and
  your own messages), which also keeps the run's lease
- reports the result when the agent's turn ends: its last reply as the summary,
  the branch and any pull request as deliveries, and the cost; or a failure

A cancel from Félagi stops the turn. Quitting Elyra Workspace hands running work
back so Félagi can give it to another runtime; with **Quit when they finish** the
run completes first and no new task is taken meanwhile. One run at a time per Mac.
**Settings → Félagi → Status** shows what the runtime is doing, and warns when
Félagi's own daemon runs on the same Mac (they would compete for the same work).

### Today's work

**Today's work** in the command palette (⌘K) shows the day at a glance:

- the Félagi issues you worked on, with the agents' time in their threads against
  the hours you have logged today, and **Log** for what is left (the agents'
  time rounded up to five minutes, less what is logged)
- other threads you worked in
- today's commits in those projects, by you

**Copy summary** puts it on the clipboard as text, ready for a standup or a
chat.

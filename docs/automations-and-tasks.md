# Automations and tasks

## Automations

An automation sends a prompt to an agent on a schedule: nightly test runs,
dependency updates, a weekly summary of open issues. Open **Automations** with
**⌥⌘A** or the calendar button at the top of the sidebar.

> Automations run while Elyra Workspace is open. A run that was due while the app
> was closed starts shortly after you open it again.

### Creating one

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

**Send to Félagi** logs the hours (replacing the running timer, so nothing is
counted twice), posts the comment and sets the status, and the thread notes what
was sent. A read-only token can show issues but not report.

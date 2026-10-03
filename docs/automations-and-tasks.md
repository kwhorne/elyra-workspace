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

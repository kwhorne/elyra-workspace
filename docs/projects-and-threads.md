# Projects and threads

## Projects

A project is a folder on your disk. Add one with **⇧⌘O**, the folder button in
the sidebar, or **File → Add Project…**. Projects are listed alphabetically, with
pinned projects first.

Right-click a project (or use its **⋯** button) to:

- **New thread**: start a thread in the project.
- **Rename and appearance…**: change the name, give it an emoji, pick an accent
  color, or put it in a *space* (see below).
- **Pin / Unpin**: keep the project at the top.
- **Reveal in Finder** and **Copy path**.
- **Change folder…**: point the project at a different folder (for example after
  you moved it). Its threads stay with it. This isn't possible while one of its
  threads is running.
- **Remove project**: removes it from Elyra. Your files are not touched.

Click the arrow next to a project to collapse it. Elyra remembers which projects
you collapsed.

### Project spaces

Spaces group projects, for example *Work* and *Personal*. Set a project's space in
**Rename and appearance…**. Once at least one project has a space, chips appear at
the top of the sidebar: **All** and one per space. Pick one to see only those
projects. Elyra remembers your choice.

### Chats without a project

**⌥⌘N** (or the speech-bubble button) starts a thread in a scratch folder,
`~/.elyra/chats`, listed as the project **Chats**. Use it for questions that don't
belong to a codebase.

## Threads

Each thread is one task with its own conversation and agent session. The sidebar
shows:

- **Pinned** threads at the top.
- Under each project, its active threads. Threads you marked as done are hidden
  behind **Show N done**.
- A **status** icon: working (spinner), needs your approval or input, failed,
  interrupted. A dot marks a thread with a reply you haven't seen yet.
- How long ago the thread was last active.

The Dock badge counts threads that need you or have unread replies.

### Thread menu

Right-click a thread (or use its **⋯** button):

| Item | What it does |
| --- | --- |
| Rename… | Give the thread a title. |
| Generate title | Let the agent write a short title from the conversation. |
| Pin / Unpin | Show it in the Pinned section. |
| Mark as done | Hide it under *Show N done*. If it was open, Elyra moves on to the next unfinished thread. |
| Mark as unread | Show the unread dot again. |
| Fork | Copy the thread into a new one that continues separately (see below). |
| Continue with ▸ | Start the same task with another provider (see *Handoff*). |
| Second opinion from ▸ | Let another provider review the last turn's changes (see *Second opinion*). |
| Compare best of N | For a best-of-N candidate: open the comparison (see *Best of N*). |
| Export… | Save the thread as a ZIP file. |
| Open terminal here | Open the thread's terminal. |
| Copy path, Copy thread ID | Copy the working folder or the thread's id. |
| Archive | Remove it from the sidebar; it can be restored. |
| Delete… | Delete it for good. If it has its own worktree, you can choose to remove the worktree too. |

Archived threads are listed under **Archived (N)** at the bottom of the sidebar.
From there you can restore or delete them.

New threads get a title from your first message. A running agent is not stopped
when you switch threads.

## Tabs and navigation

Open threads are tabs above the conversation. Elyra restores them at the next
launch. When they don't fit, scroll the tab bar sideways (the active tab is
always scrolled into view), or pick one from the **▾** menu at its end, which
lists every open tab.

When you work on several projects at once, it shows where each thread
belongs:

- The title bar (and the window title in Mission Control and ⌘\`) reads
  **project — thread**.
- Each tab starts with the project's icon or colour and its name.
- A line above the conversation shows the project, the folder the agent works
  in (click it to show it in Finder), the branch, and a **worktree** badge when
  the thread has its own worktree. The branch is checked again after each turn.

| Shortcut | Action |
| --- | --- |
| ⌘1 … ⌘8 | Go to that tab |
| ⌘9 | Go to the last tab |
| ⇧⌘] / ⇧⌘[ | Next / previous tab |
| ⌃Tab | The thread you used most recently before this one |
| ⌘[ / ⌘] | Back / forward through the threads you visited |
| ⌘W | Close the tab (the thread keeps running) |

## Split chat

**⌘\\** shows a second thread next to the active one: the one you used most
recently, or else another open tab. Use the arrows button in its header to swap
the two, and the **×** to close the split. Clicking a thread in the sidebar shows
it on the left.

## Side chats

A side chat (**⌥⌘S**) is a quick conversation that branches from the current
thread. Use it to ask about something without derailing the main thread. It
starts with the same agent, model and folder. For Claude Code, Codex, Elyra and Pi it
also starts from the main thread's conversation so far.

The side chat opens in the **Side chat** tab of the tools panel. It is not listed
in the sidebar. Its header has buttons to:

- **+** start another side chat
- **Put the last reply in the thread's composer**, to pass an answer on
- **Open as a thread**: turn it into an ordinary thread with its own tab
- **Discard** it

Side chats clean themselves up. At startup, Elyra removes side chats that were
never used, whose parent thread is gone, or that have been idle for a week.

## Fork

**Fork** (⇧⌘K, or the thread menu) copies a thread into a new one, including the
conversation. The two then continue separately:

- With Claude Code, Codex, Elyra and Pi, the new thread branches the agent's own session,
  so the agent remembers everything.
- With other providers, Elyra sends the earlier conversation along with your first
  message in the fork.

## Handoff to another provider

**Continue with ▸** in the thread menu starts a new thread with another provider,
in the same project and folder (worktree included). The message box is filled
with a summary of where the task stands: the recap from the Context tab if there
is one, plus the latest messages. Review it, add instructions, and send.

## Second opinion

**Second opinion from ▸** in the thread menu has another provider review what the
thread's last turn changed. Agents catch each other's mistakes better than their
own.

The review runs as a side chat in plan mode, so it reads but doesn't change files.
It gets your last message and the diff of the turn, and lists what it finds as
`path:line` with what is wrong and how to fix it. When it's done, the side chat's
**Put the last reply in the thread's composer** button hands the findings to the
original agent, with a note to fix what it agrees with.

## Best of N

**Best of N…** in the command palette (⌘K) gives one task to several agents at
once. Write the task, pick at least two agents and press **Start**. Each agent gets
its own thread and its own Git worktree, so they don't get in each other's way.

The comparison opens in the middle of the window. For each candidate it shows its
status, the files and lines it changed, its cost and its reply. **Open** shows the
thread, with its diff in the Changes tab. When one is done and you like it best:

1. **Use this one** commits the candidate's worktree and brings its changes into
   the project folder with `git merge --squash`. They are staged, not committed,
   so you can review them in Changes and commit them yourself. Uncommitted changes
   in the project that touch the same files may conflict.
2. **Delete the other N and their worktrees** cleans up the rest.

The candidates are ordinary threads, so you can also talk to them before you
choose. **Compare best of N** in a candidate's thread menu reopens the comparison.

## Import sessions

**⌘I** (or **File → Import Sessions…**) imports sessions you ran outside Elyra.
Choose the source at the top:

- **Claude Code**: your recent sessions from `~/.claude/projects`.
- **Codex**: your recent Codex threads. Elyra asks Codex for them, so Codex must
  be installed.

Each session is listed with title, folder, size and age. Filter by title or
folder, tick the sessions you want, and press **Import**.

- Each session becomes a thread with its conversation.
- It belongs to the project for its folder. That project is added if needed.
  Sessions whose folder no longer exists go to *Chats*.
- The thread resumes the same Claude Code or Codex session, so you can just
  continue.
- Sessions you already imported are marked and can't be imported twice.

## Export

**Export…** in the thread menu (or *Export thread* in the command palette) saves a
ZIP file with:

- `transcript.md`: the conversation as readable Markdown
- `transcript.json`: every item in the conversation, as data
- `thread.json`: the thread's settings and metadata

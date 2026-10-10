# Changes, Git and review

Elyra runs your own `git` and `gh` commands, so your Git configuration, hooks and
credentials apply.

## The Changes tab

Open it with **⇧⌘G**, or with the tools panel (⌥⌘B) and the **Changes** tab. It
shows the Git state of the active thread's folder.

### Toolbar

- **Branch menu**: the current branch. Pick another branch to switch to it. If
  your uncommitted changes would conflict, Elyra stashes them first and tells you.
  **New branch…** creates a branch from the current one and switches to it.
- **Fetch and refresh** (refresh icon). When the branch is behind its upstream, a
  **Pull** button appears (fast-forward only).
- **Push**: pushes the branch. A branch without an upstream is published to the
  remote.
- `↓2 ↑1`: commits behind and ahead of the upstream.

### Files

Files are listed in two groups:

- **STAGED**: what the next commit will contain.
- **CHANGES**: everything else.

Hover a file to **Stage** / **Unstage** it, or **Discard changes** (asks first).
**Stage all** and **Unstage all** act on a whole group. Click a file to see its
diff.

### Diffs

- **Side by side** shows old and new next to each other. Otherwise the diff is
  unified.
- **Wrap lines** and **Ignore whitespace** can be switched on.
- **⌥↓ / ⌥↑**, or the arrow buttons, jump to the next or previous change.
  **Copy diff** copies the patch.
- Very large diffs are cut off with a notice. Binary files are only listed.

Click a line in a diff to see **who last changed it** (Git blame: commit, author,
date and message). Lines that aren't committed yet have no blame. When an agent in Elyra
Workspace wrote the line, a second line says which thread (*Written by the agent in
“Fix totals” · Claude Code · 2026-10-09 14:02*); click it to open the thread. You can also write a comment about the line and press **Add to
chat**. The comment and the line's location are added to the thread's message
box, so you can collect several and send them to the agent together.

### Lines nobody has read

Elyra Workspace remembers which lines agents wrote, so it can also show which of
them nobody has looked at yet:

- A file with such lines says how many in the file list (*3 unread*), and in its
  diff each of them has a dot instead of `+`.
- **Mark as read**, at the top of the diff, marks everything agents wrote in the
  file as read. The dots go away.
- If an agent writes a line again later, it's unread again.
- Before you commit, a line above the message box says how many lines agents
  wrote in the commit that haven't been read. It's a reminder; it doesn't stop
  the commit.

Only you marking a file counts as reading it. An agent's review doesn't. The marks
stay on this Mac with the rest of the record; nothing is added to the repository.
Agents see them too: `why` says whether a person has read the line.

### Scopes

The scope menu chooses what the diff compares:

- **Working tree**: all uncommitted changes (the default).
- **Agent turns**: what one turn changed, based on the checkpoint saved before
  each turn. Use this to review the agent's work one step at a time.
- **Compare with branch or commit…**: the working tree against any branch, tag or
  commit, for example `main`.

### Committing

- **Commit staged** / **Commit all**: commits the staged files, or everything when
  nothing is staged.
- The **sparkle** button writes a commit message from the diff with the thread's
  agent. You can edit it before committing.
- The first line of a commit message follows the same
  [Conventional Commits](#conventional-commits) format as pull request titles;
  **Commit** says what to fix if it doesn't.
- **Commit and push** (⌃⌘P, from anywhere) commits and pushes in one go.

### Conventional Commits

Commit messages (their first line) and pull request titles are written as
[Conventional Commits](https://www.conventionalcommits.org): `type(scope): summary`,
for example `fix(nightwatch-exceptions): employee service create`.

- **type** is one of `feat`, `fix`, `refactor`, `perf`, `test`, `docs`, `build`,
  `ci`, `chore`, `style` or `revert`. `feat!` or `feat(api)!` marks a breaking
  change.
- **scope** is the area that changed, in lowercase kebab-case.
- **summary** is a short lowercase phrase without a trailing period; the whole
  line is at most 72 characters.

What the agent generates is written that way and tidied up (lowercase type and
scope, no trailing period). Turn the format off in **Settings → Code → Commits and
pull requests**.

## Worktrees

When you start a thread with **New worktree**, Elyra creates a Git worktree for
it under `~/.elyra/worktrees`, on a new branch. The thread, its terminal and its
Changes tab all work there. The project folder itself is not touched, so several
threads can work in parallel.

- The choice is made before the first message; it can't be changed afterwards.
- **File → Manage Worktrees…** lists the worktrees Elyra created and their
  threads. Remove ones you no longer need (their branches are kept).
- When you delete a thread that has a worktree, **Delete with worktree** removes
  both.

### Worktrees with their own running app

When [Elyra Grove](context-and-goals.md#grove) runs the project as an app, Grove
makes the worktree instead (`grove try --new`, under `~/.grove/try`): on the
same new branch, but with its **own copy of the database**, migrated, and its
**own address**, such as `shop--elyra-1a2b3c4d.test`. The thread says
where it runs, and the address opens in the thread's browser. Each thread can
change data and run migrations without touching yours, and with
[best of N](projects-and-threads.md#best-of-n) every candidate runs as its own
app you can look at.

Removing such a worktree (deleting its thread with the worktree, or in Manage
Worktrees) takes its site and database copy down with it; the branch stays. If
Grove can't start the branch, the thread gets a plain worktree and says why.
Turn this off in **Settings → Agents & MCP → Grove runs worktrees**.

## Pull requests

The **PR** tab in the tools panel works with GitHub through the
[`gh` CLI](https://cli.github.com) (installed and logged in with `gh auth login`).

If the branch has no pull request yet, you can create one:

- Write a title and description, or press **Generate** to have the agent write
  them from the branch's commits.
- Titles follow [Conventional Commits](#conventional-commits);
  **Create pull request** says what to fix if a title doesn't.
- Tick **Draft** to open it as a draft.
- Press **Create pull request**. The branch is pushed first if needed.

When the branch has a pull request, the tab is labelled **PR #n** and shows:

- status, checks and reviews
- review comments, including those on specific lines
- **Comment** to reply
- **Merge**, choosing *Squash and merge*, *Create a merge commit* or *Rebase and
  merge*
- **Ready for review** (for drafts), **Close** and **Reopen**
- **Ask agent to address feedback**: puts the review comments in the message box
  as a task for the agent

## Code review inbox

**⇧⌘R** (or the pull request button at the top of the sidebar) opens the inbox:
open pull requests and issues from the GitHub repositories of all your projects.

- Switch between pull requests and issues, and open or closed.
- Filter by project, or search by title, number, author or label.
- Select an item to read its description, checks and comments.
- **Send to agent** starts a new thread in that project with the pull request or
  issue already described in the message box, ready to send. Pull requests get a
  new worktree so the review doesn't disturb your checkout.
- **Open on GitHub** opens it in the browser.

The text of pull requests and issues is written by other people. Elyra tells the
agent to treat it as reference, not as instructions.

## Following up a review

When a thread reviewed a pull request (the agent posted a review or review
comments with `gh`), Elyra follows that pull request. Every five minutes it asks
GitHub, as you (`gh`), whether the author pushed commits since your review or asked
for a review again. When that happens:

- a card in the review thread says so (*PR #463: 2 new commits since your review,
  review requested again*), you get a notification, and it is in
  [While you were away](chat.md#while-you-were-away) and on
  [your phone](chat.md#on-your-phone);
- the pull request is marked **Changed since your review** in the code review
  inbox and listed first.

**Review the changes** on the card gives the agent only what changed since the
commit you reviewed, with your earlier comments and the replies to them. It goes
through them point by point (addressed, partly, not yet), lists anything new in
the changed code, and asks you before it posts a follow-up review or approves.
**Stop following** ends it for that thread. A pull request that is merged or
closed is no longer followed. Reviewing again (yourself or through the agent)
makes that review the new starting point.

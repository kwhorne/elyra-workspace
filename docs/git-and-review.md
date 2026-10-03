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
date and message). Lines that aren't committed yet have no blame. You can also write a comment about the line and press **Add to
chat**. The comment and the line's location are added to the thread's message
box, so you can collect several and send them to the agent together.

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
- **Commit and push** (⌃⌘P, from anywhere) commits and pushes in one go.

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

## Pull requests

The **PR** tab in the tools panel works with GitHub through the
[`gh` CLI](https://cli.github.com) (installed and logged in with `gh auth login`).

If the branch has no pull request yet, you can create one:

- Write a title and description, or press **Generate** to have the agent write
  them from the branch's commits.
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

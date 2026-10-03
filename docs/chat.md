# Working with an agent

## Sending messages

Type in the message box at the bottom of a thread and press **Enter** to send.
**Shift+Enter** starts a new line. **⌘L** puts the cursor in the message box from
anywhere.

- **↑** in an empty message box brings back the messages you sent earlier, newest
  first. **↓** goes forward again.
- What you type is kept per thread. Switching threads doesn't lose a draft.
- While the agent is working, the send button turns into **Queue**. Your message
  waits and is sent when the turn ends. Queued messages are shown above the
  message box. You can remove them, or press **Send now** to deliver one into the
  running turn (Claude Code, Codex, Elyra and Pi).
- **Stop** (or **⌃C** in the message box) interrupts the agent.

If Elyra quit while a turn was running, the thread shows *The last turn was
interrupted when the app quit* with **Resume** and **Dismiss**.

## Attachments

- **Images**: paste them (⌘V), drop them on the window, or use the paperclip
  button. They are sent to the agent with your message. Supported: PNG, JPEG, GIF
  and WebP.
- **Other files** dropped or picked are added as `@path` mentions, so the agent
  reads them itself.
- **Long pasted text** (over 4,000 characters or 60 lines) becomes a *Pasted
  text* chip instead of filling the message box. It is sent with the message.

Remove an attachment with the **×** on its chip.

## @ mentions

Type **@** to get a list of the files and folders in the project. Choose one with
↑/↓ and Enter or Tab, and the path is inserted. The list respects `.gitignore`
and matches loosely: `@thrview` finds `src/thread_view.rs`.

With Claude Code, the list also shows its subagents as `@agent-<name>`, for
example `@agent-Explore`, to ask for a specific subagent.

## / commands

Type **/** at the start of the message to see the commands:

| Command | What it does |
| --- | --- |
| `/clear` | Start a fresh conversation in this thread (the agent forgets the earlier messages) |
| `/compact` | Ask the agent to summarize the conversation to free up context |
| `/rename <title>` | Rename the thread |
| `/plan` | Switch to *Plan only* |
| `/default` | Switch back to *Ask for approval* |
| `/status` | Show the model, context use and cost |

The agent's own commands are listed after these. For Claude Code that includes
your custom commands and skills. Choosing one inserts it, and you can add
arguments before sending.

## Agent, model, effort and permissions

The row below the message box controls the thread:

- **Agent**: Claude Code, Codex, Elyra, Pi, Gemini CLI, Cursor Agent, OpenCode or your
  custom agent. You can change it only before the first message. To switch later,
  use *Continue with* ([Projects and threads](projects-and-threads.md#handoff-to-another-provider)).
  Agents you switched off in Settings are not listed.
- **Model**: the models the agent offers. Elyra learns the list from the agent
  after its first start and remembers it. At the bottom of the menu, **Star this
  model** saves the agent and model as a *preset*. Starred presets are listed at
  the top of the menu for every thread. Before the first message, picking one also
  switches the agent.
- **Account**: only shown when you set up accounts for the agent in Settings.
  Like the agent, it can be chosen only before the first message.
- **Effort**: how hard the model thinks (Claude Code: low to max; Codex: minimal
  to extra high; Elyra and Pi: off to extra high). Not every agent supports it.
- **Permission mode**:

| Mode | The agent… |
| --- | --- |
| Ask for approval | asks before editing files or running commands |
| Accept edits | edits files freely, asks before anything else |
| Plan only | reads and plans, but changes nothing |
| Full access | does everything without asking |

  Elyra and Pi don't ask for approvals; they run tools directly.
- **Local / New worktree**: where the thread works (see [Git](git-and-review.md#worktrees)).
- **Debug** (bug icon): reproduce-first mode. The agent is told to write a failing
  test or a minimal reproduction first, explain the root cause, make the smallest
  fix and show the reproduction passing. The button turns yellow while it's on.
  The change applies from the next message.
- **Context meter**: how full the model's context window is, with the cost so far
  when the agent reports it. When it's nearly full, use `/compact`.

New threads start with the agent, model, effort and permission mode you used last.

## What the agent shows

- **Text** streams in as Markdown. Its reasoning is shown in collapsible blocks
  when the agent shares it.
- **Tool calls** appear as rows: a command, a file read, a search, and so on. Click
  one to see its input and output. Edits show a diff of the change, and new files
  show their content. Running commands show their output live.
- **Task lists** the agent keeps appear as a card with progress. The latest one
  is shown in full.
- **Subagents**: work the agent hands to a subagent is grouped under one card with
  its own tool calls.
- After every turn, a line shows how long it took and what it cost.

## Approvals and questions

When the agent needs permission, an approval card shows what it wants to do:

- **Allow**: this time
- **Allow for session**: this kind of action for the rest of the session
- **Deny**

The thread is marked *needs approval* in the sidebar, and you get a notification
if you're elsewhere.

When the agent asks a question, it appears as a form. Pick an option, or several
if the question allows it, or type your own answer under *Other*. Then press
**Submit**, or **Dismiss** to not answer. Yes/no questions get **Yes** and **No**
buttons.

## Plans

In *Plan only* mode, Claude Code presents its plan as a card:

- **Approve, accept edits**: carry out the plan; edits don't need approval
- **Approve, ask for edits**: carry out the plan, asking before each edit
- **Keep planning**: stay in plan mode and keep refining

## Message actions

Hover over a message to see its actions:

- **Copy** the text.
- **Edit and resend** (your messages): puts the text back in the message box.
- **Restore files to before this message** (your messages): puts every file in
  the thread's folder back to how it was before that message was sent. Edits made
  since are undone, and files created since are deleted. The conversation and Git
  commits are not changed. Elyra asks you to confirm first.
- **Pin**: keeps the message in the Context tab ([Context](context-and-goals.md)).

Long messages are folded. Click **Show more** to expand them.

## Find in thread

**⌘F** in the message box opens a search bar for the conversation. It shows the
number of matches and jumps between them: **Enter** for the next match,
**Shift+Enter** for the previous one. **Escape** closes the bar.

## Checkpoints

Before every turn, Elyra saves a snapshot of the thread's folder, as long as it is
a Git repository. The snapshot is stored as a hidden Git reference and does not
touch your branch, index or stash. Snapshots make **Restore files** possible and
let the Changes tab show what one turn changed ([Git](git-and-review.md#scopes)).
Deleting a thread deletes its snapshots.

# Changelog

## 1.13.1

Under the hood: the agent adapters now stand on their own, so other Elyra
apps can use them. Nothing changes in how Elyra Workspace works.

- **Shared with other Elyra apps.** The provider adapters no longer need the
  workspace's own model: the types they share live in a small crate of their
  own. A host can start an agent through a launcher (such as a sandbox), give
  it only the environment it names, keep Claude Code from switching on Full
  access, or withhold an ACP agent's file access. Elyra Workspace keeps its
  defaults.
- **Attachments are kept with a message.** A message can now carry attachments
  as copies kept with the thread, ready for what comes next.

## 1.13.0

Ask the agent that wrote a line why, and Laravel Boost made part of the workspace: one
setup for every agent, one rule book, and app errors that lead back to the
instruction behind them.

- **Ask why.** Click a line an agent wrote in the Changes tab and press **Ask
  why**. A read-only side chat opens with the agent that wrote it, picked up
  where that turn ended: it has that conversation and nothing after it, what the
  turn changed in the file and the code around the line now. So it answers from
  what it knew and decided then, and says if it would do it differently today.
  Agents can ask too, with the gateway's new `ask_author` tool. See
  [Ask why](docs/git-and-review.md#ask-why).
- **Made for Laravel Boost.** See
  [Laravel Boost](docs/context-and-goals.md#laravel-boost).
  - The Context tab says whether Boost is set up, and what to run if it isn't.
  - `php artisan boost:update` no longer stops sharing `.mcp.json`. Allowing
    the file remembers each server, so the ones that stayed the same stay
    shared, and only new or changed ones wait for you.
  - New worktrees get what Git leaves out: `.env`, `.mcp.json`, the guideline
    files and skills, and `vendor/` as an APFS clone. So `php artisan` and
    Boost work there too.
  - Rules agents learn go to Boost's `.ai/rules`, for the files they apply to,
    along with its index. There is one rule book for your team.
- **App errors lead to the instruction behind them.**
  - When the Laravel app logs an error, the thread shows the line in your code
    it failed on and the thread that wrote that line.
  - **Ask why** asks that agent about it.
  - Agents can look it up with the new `error_origin` tool.
- **Reviews know the tools.** A second opinion is told about Boost's tools, or
  `rata mcp dev` in an Elyra Framework app. The context inspector shows how much
  of the agent's setup Boost's guidelines take.
- **Fix:** a Grove site without a folder (a proxy to a port) no longer counts as
  serving every folder. Before, a new worktree in a project outside Grove could
  be made as a Grove branch of that site.

## 1.12.1

A clearer sidebar.

- **Projects and threads tell apart at a glance.** Projects read as headings,
  and their threads hang off them along a guide line in softer text. Unread
  threads stay bold and the selected one keeps its highlight.

## 1.12.0

A branch can read as the story of what was asked: one commit per instruction.

- **Restack by instruction.** One click in the PR tab rewrites a branch as one
  commit per instruction, in the order they were asked, each titled with the
  message and dated when it was asked, plus one for what no agent wrote. Your
  files stay exactly as they are, a dialog shows what will change first, and
  **Put the old commits back** undoes it. Agents can preview it with the
  gateway's new `restack_preview` tool. See
  [Restack by instruction](docs/git-and-review.md#restack-by-instruction).
- **What a branch does, also without GitHub.** The PR tab shows the branch by
  instruction, with **Review step by step** and **Restack by instruction**, also
  in projects without a GitHub remote: they only need Git.

## 1.11.0

Review a branch as the story of its instructions, and resolve conflicts knowing
what each side was for.

- **Review a branch step by step.** Go through a branch one instruction at a
  time: what was asked, what the agent said it did, and the lines it wrote with
  the unread ones marked. **Mark as read and next** marks them and moves on.
  Open it from the PR tab or the command palette. See
  [Review step by step](docs/git-and-review.md#review-step-by-step).
- **Merge conflicts that know why.** When a merge leaves conflicts, **Resolve,
  knowing why** in Changes gives the agent each conflict with the instruction
  behind each side, so it keeps what both were asked to do. Agents can ask with
  the gateway's new `conflict_context` tool. See
  [Merge conflicts](docs/git-and-review.md#merge-conflicts).

## 1.10.0

Take back one instruction without losing what came after it.

- **Undo an instruction.** Take back what one message had an agent change, even
  after other turns came after it: its edits go, the files it created are
  removed and the ones it deleted come back, while later work stays, also in the
  same files. Files that changed again in ways that don't merge are left as
  they are, and **Ask the agent to finish** hands them to the agent; **Put it
  back** brings the undone changes back. Elyra now also keeps a snapshot of the
  folder when each turn ends, so an undo stops exactly where the turn did.
  Agents get the gateway's new `undo_instruction` tool. See
  [Undo an instruction](docs/chat.md#undo-an-instruction).

## 1.9.0

When the checks fail, find the instruction that broke them in one click, and
see how each agent does on your own code.

- **Who broke it?** When the checks fail, one click finds the instruction that
  made them fail: the checks run against the state before each earlier turn in
  the folder, across threads and agents, in a temporary copy (your working folder
  isn't touched), halving the range until the turn where they went from green to
  red. A card names the message, its thread and agent, with the failing output,
  and sends the failure to that thread on request. Agents can ask with the
  gateway's new `who_broke_it` tool. See
  [Who broke it?](docs/context-and-goals.md#who-broke-it).
- **How the agents do on your code.** Usage statistics shows, per agent and
  model, how often the checks were green on the first try and in the end, how
  many automatic fixes it took and what an instruction cost, from your own
  threads. A new thread shows the same for its project when there's enough to
  compare. See
  [How the agents do on your code](docs/context-and-goals.md#how-the-agents-do-on-your-code).

## 1.8.0

A branch reads as the instructions behind it, and a pull request can say what
was asked for without asking a model.

- **What this branch does.** The PR tab shows what a branch adds, grouped by the
  instruction behind it: each message with its thread, agent, files, whether the
  checks passed afterwards, what it cost and how many of its lines nobody has
  read, and the lines no agent wrote last. It works for every agent and across
  threads, from the record Elyra keeps of which message wrote which lines.
  **From instructions** turns it into a pull request title and description
  without a model, and agents get it with the gateway's new
  `changes_by_instruction` tool. See
  [What this branch does](docs/git-and-review.md#what-this-branch-does).

### Fixed

- **Checks after a turn that changed nothing.** When a turn ended, recording
  its lines and deciding whether to run the checks both took a snapshot of the
  working tree at the same moment, and could pick the same temporary file. One
  of them then failed, so the checks sometimes ran (and an automatic fix was
  sent) after a turn that changed nothing, or a line record went missing.

## 1.7.0

Faster all round, most of all with long threads and agents at work, and Go to
symbol.

- **Go to symbol (⌘T).** Jump to a function, method, class or type by name,
  with fuzzy matching like Find file, from the same symbol index the agents use.
  Picking one opens its file at that line. See
  [Go to symbol](docs/files-and-search.md#go-to-symbol-t).
- **Faster, especially with long threads and agents at work.** The thread now
  draws only the rows in view, so a thread with thousands of entries scrolls and
  streams as smoothly as a short one; it still follows the end while you're at
  the bottom. While an agent writes, only its thread redraws: the side panel,
  the sidebar and other threads are drawn from cache, a streamed reply is parsed
  only as far as it is new, and threads in the background no longer redraw the
  window. Searching, the usage statistics and other slow reads run off the main
  thread, the title bar and sidebar stopped doing work on every frame, the
  terminal cursor only blinks while the terminal has focus, and closed panels
  stop polling. Writes to the database no longer wait for the disk each time.

## 1.6.0

Find anything you or the agents said or did, in every thread, in an instant.

- **Search threads (⌥⌘F).** Find anything in any thread: your messages, the
  agents' replies, their tool calls and what the tools printed. Every word must
  occur and matches word beginnings, "quotes" make a phrase, and results come
  best first, grouped by thread, with the matching words marked. Filter by
  project, include archived threads or leave tool output out; a click opens the
  thread at the entry. The command palette's message search uses the same index
  and now ranks its hits. Agents can search too, with the gateway's new
  `search_threads` tool. See
  [Search threads](docs/files-and-search.md#search-threads-f).

## 1.5.1

Gemini CLI as a full member: it gets the project's AGENTS.md and the shared
skills like the other agents.

- **Gemini CLI gets the project's AGENTS.md.** Gemini CLI reads only GEMINI.md
  by default, so in projects that keep their agent instructions in AGENTS.md (or
  CLAUDE.md) Gemini threads missed them. They now get the file along with their
  instructions, unless the project has a GEMINI.md or your Gemini settings
  already name the file.
- **Skills in `.gemini/skills`** are shared with every agent too, and each agent
  gets only the skills it doesn't find itself: Claude Code now also sees the
  skills in `.agents/skills` and `.gemini/skills`.
- **Learned rules go into GEMINI.md** in projects that use it rather than
  AGENTS.md or CLAUDE.md.

## 1.5.0

See what fills an agent's context and what it costs, and full support for ACP
agents such as Gemini CLI: they now get the project's instructions, and side
chats, forks and plan mode behave as they do with the other agents.

- **What's in the context.** The Context tab starts with what fills the agent's
  context window: the agent's own setup, your messages, its replies, tool calls
  and tool results, with sizes, shares and about what each cost in the last
  turn. It lists the largest entries (click one to find it in the conversation),
  says when a single tool result takes up a lot of room, and has **Compact** and
  **Clear** at hand. Click the context meter to open it. See
  [In the context](docs/context-and-goals.md#in-the-context).

### Fixed

- **ACP agents get the instructions.** Gemini CLI, Cursor Agent, OpenCode and
  other ACP agents never received the project instructions, debug mode or the
  shared skills. They now lead the first message of each new session.
- **ACP side chats no longer share their parent's session.** A side chat on an
  agent that can't branch a session continued the main thread's own session,
  so the two wrote into one conversation. It now starts its own, with the
  conversation so far.
- **Codex forks natively.** Fork branched the Codex thread only for side chats;
  it now does for **Fork** too, instead of sending the conversation as text.
- **Plan mode doesn't write through ACP.** An ACP agent could still write files
  in *Plan only* by calling Elyra's file API directly. Those writes are refused,
  and relative paths resolve in the thread's folder.
- **Titles, commit messages and recaps respect Settings.** They used the
  agent found on `PATH` and ignored a custom executable, its environment and the
  thread's account, and fell back to switched-off agents. They now use the same
  settings as threads, skip disabled agents, and can fall back to Codex and
  Gemini CLI too.
- **Compact only where it works.** The Context tab's **Compact** is disabled for
  agents Elyra can't ask to compact.

## 1.4.0

Agents that know about each other, and code you know you've read: parallel
agents see what the others are changing, and Changes shows which lines agents
wrote that nobody has looked at yet.

- **Lines nobody has read.** Changes marks the lines agents wrote that nobody
  has read yet: a count per file, a dot on each line in the diff, and a reminder
  before you commit them. **Mark as read** clears a file; an agent's later
  rewrite of a line makes it unread again. `why` tells agents whether a person
  has read a line. See [Git and review](docs/git-and-review.md#lines-nobody-has-read).
- **Agents side by side.** When several threads work in the same repository,
  each agent learns what the others changed during the last hour: a short list in
  front of every message, and the gateway's new `intent_board` tool. Two threads
  that change the same file are both told, with the functions they share and
  whether they work in the same folder or meet at merge. Best of N candidates and
  side chats are left out. See
  [Agents side by side](docs/projects-and-threads.md#agents-side-by-side).

## 1.3.1

Updates keep working whichever of the two Apple teams signs a release.

- **Updates from either release team.** Releases are signed by GETS AS or by
  Knut Horne, and an official copy now accepts an update signed by either, so a
  release can move between them without anyone having to reinstall by hand.
  1.3.0 was the first release signed by Knut Horne: copies on 1.2.0 or older
  still need to install 1.3.0 or later from the disk image once. A copy you
  signed yourself still only takes updates from your own team.

## 1.3.0

Agents understand the code they work in: every agent can look up where a symbol
is defined and who calls it, and why a line exists, back to the thread and the
message that had an agent write it.

- **Where code came from.** Every turn now records the lines it added, so a line
  leads back to the thread and the message that had an agent write it, for every
  agent, also when it committed itself. Agents ask with the gateway's `why` (this
  line) and `history` (this file or symbol); click a line in Changes and the
  thread behind it is one click away.
- **Symbol lookup for every agent.** The gateway's new `symbols` tool answers
  "where is this defined, who calls it" from a tree-sitter index of the project
  (Rust, TypeScript, JavaScript, Python, Go, PHP, C): `file:line` with
  signatures and the calling function, so agents read only what they need.
  Claude Code, Codex, Elyra, Pi and ACP agents all get it. The index comes from
  Elyra Desktop.

## 1.2.0

After you review a pull request, Elyra keeps an eye on it: when the author
pushes changes or asks again, you hear about it, and one click has the agent
check only what changed against what you asked for.

- **Following up a review.** After a thread reviewed a pull request, Elyra
  follows it: new commits since your review, or a review asked for again, bring a
  card to the thread (and a notification, the away summary, your phone), and the
  inbox marks it **Changed since your review**. **Review the changes** gives the
  agent only what changed since the reviewed commit, with your earlier comments
  and the replies, to check point by point what was addressed.

## 1.1.0

Agents that test what they built: a flow they walk through in the browser
becomes a journey that is replayed after later changes, a second fix gets a
stronger model, and best of N recommends the candidate whose checks pass. Away
from the desk, a summary waits for you when you come back, and your phone can
approve and answer. Corrections become rules every agent follows.

- **While you were away.** Back at the window after a while, a summary shows what
  the threads did meanwhile: what needs you, what failed, what finished and what
  is still working, with the cost. Click a row to open the thread; the command
  palette shows it again.
- **Answer from your phone.** Turn on Settings → General → Phone and threads that
  need you or finish while you're away push to your phone through ntfy, with
  Allow, Deny and answer buttons; reply in your own words to answer a question
  or send a message. Works for every thread, also automations and Félagi runs.
- **Rules agents learn.** Correct an agent and it can propose the correction as a
  rule; one click adds it to AGENTS.md (or the project's instructions).
- **A stronger second fix.** When the checks or journeys still fail after a first
  automatic fix, the next one gets the agent's escalation model (Settings →
  Providers, e.g. `opus`) and its highest effort, then the thread goes back to
  its own model. Cheap model for the easy turns, strong model when it counts.
- **Best of N picks with you.** The comparison shows whether each candidate's
  checks and journeys passed, and recommends one with a reason: green first, then
  the smallest change, then the lowest cost.
- **Browser journeys.** An agent that tried a flow in the browser (open a page,
  click, fill in, press, wait) can save it as a journey in the project's
  `.elyra/journeys`, with what must be on the page when it works. Replay journeys
  from the Context tab, or switch on **Replay after each turn**: after every turn
  that changed files they run in the thread's browser, and a broken flow goes back
  to the agent like a failing check. UI regressions get caught without anyone
  writing browser tests.

## 1.0.0

Elyra Workspace 1.0. Threads for every task with Claude Code, Codex, Elyra, Pi and
ACP agents; Git, review, a browser, a terminal and your files in one window; and
now agents whose work is checked, who can try what they built, and who share one
setup. Every push runs the real app through end-to-end scenarios, and an update
that doesn't start falls back to the previous version by itself.

- **Narrow windows.** The conversation keeps at least 400 points: the tools
  panel gives way first. Notices wrap instead of running into the tools panel,
  long tab titles end in "…", and the tools panel's tabs scroll with a **▾** list
  when they don't fit. Automations explains its empty detail column.
- **One setup for every agent.** A project's `.mcp.json` servers and its skills
  (`.claude/skills`, `.agents/skills`, also in your home folder) now reach Codex,
  Elyra, Pi and ACP agents too, not only Claude Code. `.mcp.json` runs commands
  from the repository, so it's shared only after you allow it in the Context tab
  (and again when it changes).
- **Agents propose automations.** Ask for something on a schedule (*"check our
  dependencies every weekday night"*) and the agent proposes it as a card in the
  thread: schedule and next run, agent, project and prompt, with **Create**,
  **Edit…** and **Dismiss**. Nothing is scheduled unless you accept.
- **Agents use the page.** Besides looking at a local page, agents can now
  click (by selector or visible text), fill in fields (by label too; selects and
  checkboxes), press keys and wait for something to appear, so they can try the
  flow they just built. The first time, you're asked (*Allow once* or *Allow for
  this thread*); **Take over** in the browser withdraws it. Full access threads
  aren't asked.
- **Back to the previous version.** Each update keeps the version it replaced.
  If a new version fails to start twice in a row, Elyra Workspace goes back to
  the previous one by itself, says so, and skips the broken version until the
  next release. Command palette → **Go back to the previous version…** does it
  by hand.
- **Checks see every change.** Whether a turn changed files is now judged
  against the snapshot taken before the message reached the agent, so a quick
  agent can no longer slip a change past the checks.
- **Resuming checks what was done.** An interrupted turn that is resumed asks
  the agent to check what already happened before it continues, so nothing is
  done twice.
- End-to-end scenarios run on every push: the app headless, driven through the
  agent gateway by a scripted agent that crashes, fails and stalls on cue.

## 0.10.1

- **Tabs stay in their place.** With many open threads the tab bar no longer
  runs into the tools panel: it scrolls, keeps the active tab in view, and a
  **▾** menu at its end lists every tab.

## 0.10.0

- **Done means green.** Give a project a check command in the Context tab (Elyra
  suggests one: `cargo test`, `php artisan test`, `npm test`…). After every turn
  that changed files it runs; if it fails, the output goes back to the agent
  (twice by default, Settings → Agents & MCP), and the thread is only done when
  the checks pass. Notifications, Félagi reports and `wait_for_thread` wait for
  them.
- The budget no longer shows "$-0.00 spent".

## 0.9.1

- **Large file editor.** The expand button in the Files tab (or ⌥⌘E) slides the
  file tree and editor in from the right over half the window, instead of the
  narrow tools panel. Drag its edge to resize it.

## 0.9.0

- **Quit when the agents finish.** ⌘Q while agents are working offers to hide
  the window and quit once they are done, instead of stopping them. Coming back
  calls it off; no new Félagi tasks or automation runs start while it waits.
- **The Dock brings the window back.** Closing the window keeps agents working;
  click Elyra Workspace in the Dock to open it again.
- **Continue all interrupted turns.** After a quit or crash that cut turns off, a
  notification at launch continues them all at once.
- **Agents ask before using another thread's browser.** Opening or reloading a
  page in another thread's browser now asks you first, like messaging,
  stopping, renaming or archiving another thread.

## 0.8.2

- **See which project you're in.** The title bar reads *project — thread*, tabs
  start with the project's icon or colour and name, and a line above the
  conversation shows the project, the folder (click to show it in Finder), the
  branch and whether the thread has its own worktree.

## 0.8.1

- **Elyra Workspace as a Félagi runtime.** Turn on Settings → Félagi → Run
  Félagi's agents on this Mac (with a daemon token and a unique machine name),
  choose the Mac as an agent's runtime in Félagi, and the work Félagi gives that
  agent runs here as a thread in the matching project and its own worktree. You
  can watch and step in; what the agent does is streamed to Félagi, and the
  result (summary, branch, pull request, cost) is reported when its turn ends.

## 0.8.0

Grove, Workspace and Félagi, closer together:

- **Pictures in Félagi reports.** The latest before and after pictures of the
  page are attached to the report's comment.
- **Pull requests name their issue.** A thread on a Félagi issue makes pull
  request titles like `fix(acm-231): …` and ends the description with
  `Félagi: ACM-231`; the report links to the pull request.
- **File a server error in Félagi.** From the error chip, the newest server error
  Grove recorded becomes a Félagi issue (exception, with Grove's explanation),
  and the thread is linked to it.
- **Today's work** (command palette): the issues you worked on with the agents'
  time against the hours logged and a click to log the rest, other threads, and
  today's commits; copyable for a standup.

## 0.7.0

- **Félagi tasks and time.** Connect [Elyra Félagi](https://github.com/kwhorne/Felagi)
  in Settings → Félagi (the token is kept in the macOS Keychain), and the task
  board gets a Félagi tab with the issues assigned to you. **Start** opens a
  thread on an issue, moves it to In progress and starts Félagi's timer; the
  thread's banner shows the issue and the timer, and **Report…** writes back
  what was done (drafted by the agent), the status and the hours.

## 0.6.0

More with [Elyra Grove](https://github.com/kwhorne/grove):

- **Worktrees with their own running app.** In projects Grove runs as an app, a
  thread's new worktree comes from Grove (`grove try --new`): its own copy of
  the database, migrated, and its own `.test` address, which opens in the
  thread's browser. Best of N candidates each run as their own app. Removing the
  worktree takes the site and database copy with it.
- **The database in every checkpoint.** In Grove apps the checkpoint before each
  turn also snapshots the database (SQLite, MySQL, PostgreSQL or ElyraSQL), and
  *Restore files* puts it back, undoing the agent's data changes and migrations
  with its code. The latest 10 per thread are kept.
- **Did the fix work?** Server errors you send to the agent are replayed when
  its turn ends (`grove replay --same-data`), and the conversation says *was
  500, now 200 ✓* or *still 500*.
- The error chip, the Grove box in the Context tab and the browser's start page
  follow a worktree's own site.

## 0.5.0

Working with [Elyra Grove](https://github.com/kwhorne/grove), the local
development environment:

- **Server errors in the error chip.** Requests Grove records with a 5xx status
  for the project's app show up next to browser errors; *Add to message*
  attaches Grove's explanation: the request, its SQL and mail, and the stack
  trace from the error log.
- **Grove's tools for agents.** In projects Grove runs as an app, agents get
  Grove's MCP server (read-only) next to the agent gateway, so they can look up
  recent requests, logs and the database schema themselves. Turn it off in
  Settings → Agents & MCP.
- **Grove in the Context tab.** The app's `.test` address, its `grove dev`
  processes with Start and Stop, and caught mail; the Browser tab's start page
  offers the address too.
- MCP servers given to agents can now be local commands as well as HTTP servers.

Also new:

- **Material theme**, after [Material Theme for VS Code](https://marketplace.visualstudio.com/items?itemName=vihuvac.material-theme-for-vscode):
  blue-grey surfaces, the Material colours and a teal accent. Settings →
  Appearance.

## 0.4.0

- **Point at an element.** The target button in the Browser tab lets you click
  something on the page; your message gets a picture of it and its selector,
  position, text, computed styles and HTML, so the agent knows exactly which
  element you mean.
- **Agents ask before changing another thread.** The first time an agent sends a
  message to, stops, renames or archives another thread through the gateway, you
  choose Don't allow, Allow once or Always allow; remembered pairs can be
  forgotten in Settings → Agents & MCP. Threads an agent starts itself are its
  own and need no asking.
- **Updates wait for your agents.** Restarting for an update while agents are
  working offers *Restart when they finish*, which restarts by itself once no
  turn is running, as well as *Restart now*.

## 0.3.1

- **Conventional Commits.** Generated commit messages and pull request titles
  follow `type(scope): summary`, such as
  `fix(nightwatch-exceptions): employee service create`, and *Commit* and
  *Create pull request* check the format and say what to fix. On by default;
  turn it off in Settings → Code → Commits and pull requests.

## 0.3.0

- **Best of N.** Give one task to several agents at once (command palette →
  *Best of N…*), each in its own worktree. Compare their changes, cost and
  replies side by side, and bring the one you pick into the project, staged for
  review. See [Projects and threads](docs/projects-and-threads.md#best-of-n).
- **Second opinion.** *Second opinion from ▸* in the thread menu has another
  provider review the last turn's diff in a read-only side chat; one click hands
  its findings back to the original agent.
- **Errors from the browser, one click away.** New console errors, uncaught
  exceptions and failed requests on a local page show up as a chip above the
  message box; *Add to message* attaches them.
- **Pictures before and after a turn.** With a local page in the Browser tab,
  each turn gets a picture of the page from before and after it in the
  conversation.
- **Budgets.** Set a spending limit for a thread in the Context tab. At 80% Elyra
  says so; at the limit goals pause and ask you.
- The browser now records uncaught exceptions and unhandled promise rejections
  for agents too, with their messages.

## 0.2.3

- Elyra threads have the gateway's `mcp__elyra__…` tools from their first
  message (with Elyra 0.9.47 or later). The gateway token now reaches Elyra in
  its own environment variable, so Elyra's cached tool list survives the new
  token each session.
- The command palette (⌘K) scrolls to keep the selected row in view when you
  move with the arrow keys.

## 0.2.2

- Elyra threads get the agent gateway too (Elyra 0.9.46 or later), as
  `mcp__elyra__…` tools. Pi still doesn't.

## 0.2.1

First release with the browser (0.2.0 was tagged but never published).

- **Browser.** Each thread has a web browser in the tools panel (**⇧⌘B**, or
  click a local server in the Context tab) for looking at the app you're
  building. See [Browser](docs/browser.md).
- With the agent gateway on, agents can open their thread's local dev server in
  it and inspect the page: DOM outline, elements and computed styles, console,
  network calls and a screenshot. Only pages served from this Mac (localhost,
  `*.test`, `*.local`) can be opened or read.
- The browser and its agent tools are adapted from Litr by Wirelabs AS, used
  under the MIT licence with permission.

## 0.1.5

- **Help → Elyra Workspace Documentation** (and "Documentation" in the command
  palette) opens the user guide on
  [elyracode.com/docs/workspace](https://elyracode.com/docs/workspace).
- Update checks keep working when GitHub's API limit is reached (60 requests an
  hour per network, shared by everyone on it): the app then reads the latest
  version from the release page instead.

## 0.1.4

No changes to the app itself.

- Releases are now built, signed, notarized and published by GitHub Actions
  when a version tag is pushed; this is the first release made that way.
- New README with a screenshot, and a development guide
  ([elyracode.com/docs/workspace/development](https://elyracode.com/docs/workspace/development))
  for building and releasing.

## 0.1.3

- Maintenance release with no functional changes, published to verify that
  0.1.2 updates itself automatically.

## 0.1.2

- **Automatic updates.** New versions are downloaded and verified in the
  background (checksum, same Developer ID team, notarization, version) and
  installed when you restart or quit; the app relaunches itself. Checks run at
  launch and every six hours. Settings → General can turn off automatic
  downloads. Apps running from the disk image or a read-only folder still get a
  download link.

## 0.1.1

- **Codex** as a provider through `codex app-server`: streaming, reasoning,
  commands with live output, diffs, plans, questions, approvals mapped to Codex's
  approval policy and sandbox, steer, compact, models, effort, native fork and
  resume, project instructions, and the agent gateway (MCP)
- Import Codex threads alongside Claude Code sessions (⌘I)
- Fix: the shortcuts and command palette entries for fork (⇧⌘K), import (⌘I),
  the task board (⌥⌘T), automations (⌥⌘A), statistics and export were missing

## 0.1.0

First release, for macOS (Apple Silicon). See [docs/](docs/README.md) for the user
guide.

- Projects and threads with tabs, pinning, spaces, archive, fork, side chats,
  split chat, handoff to another provider, import from Claude Code and ZIP export
- Agents: Claude Code, Elyra, Pi and Agent Client Protocol agents (Gemini CLI,
  Cursor Agent, OpenCode, or any ACP command), with accounts per provider
- Conversation: streaming, tool calls with diffs, subagents, task lists, plans,
  questions, approvals, attachments, `@` mentions, `/` commands, queue and steer,
  checkpoints with restore, find in thread, debug mode
- Git: staging, commits with generated messages, branches, sync, per-turn diffs,
  blame, line comments to the agent, worktrees per thread, pull requests and a
  code review inbox (via `gh`)
- Files tab with editor, command palette, file finder and content search,
  external editors
- Terminal with full TUI support, splits, scrollback search and "add to chat"
- Context tab: notes, pinned messages, recap, project instructions, local servers
  and thread goals
- Automations on schedules, a task board, usage statistics
- Agent gateway (MCP) for agents and external clients, with an audit log
- Settings: themes (incl. custom), fonts, terminal, providers, shortcuts

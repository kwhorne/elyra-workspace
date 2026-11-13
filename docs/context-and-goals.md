# Context, notes and goals

The **Context** tab in the tools panel (**⇧⌘I**) collects everything around the
active thread.

## In the context

The top of the tab shows what fills the agent's context window. It counts the
conversation since the last **Clear**, next to what the agent last reported:

- A bar and one row per part: the **agent setup** (its system prompt, tool
  definitions and instructions, i.e. whatever the conversation doesn't account
  for), **your messages**, **agent replies**, **tool calls** and **tool results**.
  Each row has its size, its share of the context and roughly what carrying it
  cost in the last turn.
- **Largest**: the biggest single entries, such as a long log a command printed.
  Click one to scroll the conversation to it.
- When the agent has compacted earlier parts, a line says roughly how much.
- When one tool result takes a large part of the context, a hint says so. The
  agent reads the whole context again on every call it makes, so a large result
  costs on every step until the conversation is compacted.

**Compact** has the agent summarise the conversation so far. **Clear** starts a
new conversation in the thread; the transcript stays. Neither is available while
a turn runs.

The numbers are estimates (about four characters a token). Prices are each part's
share of what the last turn cost. Agents that don't report their context use
(some ACP agents) get the conversation estimate only. Click the context meter
below the message box to open this tab.

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

### Rules agents learn

When you correct an agent (*"no, we always validate with Form Requests"*), it can
propose that as a rule: a card in the thread with the rule and what taught it.
**Add to AGENTS.md** puts it under *Learned rules* in the repository's AGENTS.md
(or CLAUDE.md or GEMINI.md, when that is what the project has), so every agent and your team
get it; commit it with your changes. **Add to project instructions** keeps it in
Elyra only. **Save as check** asks the agent to turn it into an
[invariant](#invariants-rules-that-get-checked), when a pattern or a short
command can catch a mistake against it. **Dismiss** drops it.

**Rules that may be stale.** Rules name files, folders and code in backticks
(*Put VAT in \`TaxService\`*). When what a rule names is gone from the repository
(no file has that path, no file mentions that name any more, or a Boost rule's
glob matches no file), the Context tab lists the rule under *Rules that may be
stale*, with what is gone. Agents read rules on every turn, so a stale one
misleads them and costs context. **Remove** takes its line out of the file;
**Where it came from** opens the thread that proposed it, to see what taught it.
The generated guidelines of Laravel Boost and Elyra Framework aren't checked.

In a project with [Laravel Boost](#laravel-boost)'s project rules, the rule goes
to `.ai/rules` instead, where Boost keeps them: the agent says which files it
applies to (*app/Http/Controllers/\*\**), the card shows it, and the rule is added
to that area's file (`.ai/rules/controllers.md`, with the globs in its
frontmatter) and to `.ai/rules/index.md`, the index Boost tells agents to read
before they edit. So there is one rule book, whichever tool wrote the rule.

### Invariants: rules that get checked

A rule in AGENTS.md is read, and sometimes forgotten; an invariant is checked.
The hardest mistakes are the ones left out of new files: a new API route without
the tenant scope you asked for last week. An invariant is a JSON file in the
repository's `.elyra/invariants`, so your team and other tools have it too:

```json
{
  "name": "API controllers scope queries to the tenant",
  "paths": "app/Http/Controllers/Api/**/*.php",
  "require": "tenant_id",
  "forbid": "\\bdd\\(",
  "why": "Customers must never see another tenant's orders."
}
```

- `paths`: the files it applies to, a glob (`**/` any folders, `*`, `{ts,tsx}`);
  left out, all files.
- `require`: a regular expression each of those files must match.
- `forbid`: one no line a turn **adds** to them may match (lines already there
  don't count).
- `command`: a shell command that must succeed, run at the top of the project
  with the files in `ELYRA_FILES`, one per line (under a minute).

After every turn that changed files, the invariants covering what the message
changed (its automatic fixes included) are checked, in milliseconds and without a
model, before the [checks](#checks-after-each-turn): **2 invariants held**, or
**Invariant broken** with each file and line. A broken one goes back to the agent
like failed checks, with the rule and why it matters, and the slower checks wait
until it holds.

Agents propose them with the gateway's `propose_invariant`, when your correction
is something a pattern can catch, or when you press **Save as check** on a
proposed rule. The card shows what it checks and tries it on the code as it is
(*The code doesn't hold it in 1 place yet*); **Save as check** writes the file,
to commit with your changes.

## Checks after each turn

*Done means green.* Give the project a check command (tests, lint, or both,
such as `cargo test` or `vendor/bin/pint --test && php artisan test`). Elyra
suggests one from the project's files; **Use …** fills it in, and **Run now**
tries it.

- After every turn that changed files (Elyra compares the Git working tree
  before and after), the command runs in the thread's folder, with your login
  shell and `CI=1`. A turn that changed nothing isn't checked.
- The thread stays busy while the checks run. **Checks passed** or **Checks
  failed** appears in the conversation; click it for the output.
- When they fail, the end of the output goes back to the agent to fix
  (*Sent the failures to the agent*), and the checks run again. After **Settings
  → Agents & MCP → Automatic fixes when checks fail** (2 by default) the thread
  stops, marked failed, and you are told. 0 only reports the failure. No
  automatic fix is sent when you have queued a message or the budget is used up.
- **A stronger fix**: when a first automatic fix didn't make them pass, the next
  one gets the agent's **Escalation model** (Settings → Providers, e.g. `opus`)
  and its highest effort; the thread goes back to its own model afterwards. With
  no escalation model set, only the effort goes up.
- **Stop** stops the checks too. A check is stopped after 20 minutes.
- Notifications, Félagi reports and `wait_for_thread` wait for the checks, so
  *finished* means the checks passed.

The command applies to every thread in the project. Empty turns it off.

### Test the tests

Green checks say the code passes the tests, not that the tests would notice if
it were wrong. Agents often write tests for the happy path only, so half the
conditions they add could be turned around without a test failing. So once the
checks pass after a turn, Elyra tests the tests, without asking a model:

- It finds the conditions in the lines the instruction added (with tree-sitter,
  in Rust, TypeScript, JavaScript, Python, Go, PHP and C, and the scripts of
  Vue and Svelte components) and picks up to four
  small changes, spread over the files: a flipped comparison (`>` to `<=`,
  `===` to `!==`), `&&` for `||`, a dropped `!` or `not`, `true` for `false`.
  Test files are left alone.
- It runs the check command with each change in turn, in a temporary copy of
  the folder as the turn left it (your files aren't touched). A run that takes
  over three times as long as the checks did counts as caught.
- **The tests caught all 4 changes**, or **The tests missed 1 of 4 changes**
  with the line and the change that got through (*`&&` → `||`: no test
  failed*). **Ask for tests** has the agent add tests that fail with those
  changes, without changing the code (*Asked the agent for tests that catch the
  changes*). Once its checks pass, the same changes are tried again, so you see
  whether the new tests catch them. **Stop** stops it.
- The lines that got through are also marked in Changes, with **Ask for a
  test** for each ([Lines no test guards](git-and-review.md#lines-no-test-guards)).

It only runs when the checks took under 90 seconds, since they run once per
change. In a [duel](projects-and-threads.md#red-vs-blue) it tests Red's tests
against all of Blue's code, and what gets through goes to Red for one more
round instead of **Ask for tests**. **Settings → Agents & MCP → Test the
tests after green checks** sets how many changes are tried (4 by default; 0
turns it off).

### When an agent goes in circles

Some turns never get to the checks: the agent runs the same failing command
again and again, or one call after another fails, and the turn burns context
and money. While a turn runs, Elyra watches its tool calls (no model is asked)
for two signs:

- the same call failing the same way three times (numbers such as times and
  line numbers aside), so what the agent changed in between made no difference;
- six tool calls in a row failing, with nothing working in between (each
  subagent counts on its own).

Calls you turned down don't count, and neither do Red's in a
[duel](projects-and-threads.md#red-vs-blue), whose tests fail on purpose. Then **The agent is going in
circles** appears in the conversation with the reason and the end of the last
failure, and you are notified:

- **Start over** stops the turn, puts the files (and the database, when the
  checkpoint has it) back to how they were before it, and sends the agent at
  the same request again (*Sent the agent at it again*), with what went wrong
  and the **Escalation model** and highest effort, as a second fix gets. The
  thread goes back to its own model afterwards.
- **Stop** stops the turn and leaves the files as they are.
- **Let it run** dismisses it for the rest of the turn.

**Settings → Agents & MCP → When an agent goes round in circles**: *Ask me*
(default), *Start over on its own*, or *Do nothing*. A turn that was started
over doesn't count as green on the first try in the
[agents' numbers](#how-the-agents-do-on-your-code). A start-over is skipped when
the thread's budget is used up.

### Which agent for this part of the code

The numbers above are for the whole project, but agents differ within one: one
may do well in the Rust core and not in the CSS. So every instruction's outcome
is also tied to the folders its turns wrote in (from the line record, two levels
deep, such as `app/Http` or `crates/elyra-app`).

When you write a new thread's first message, Elyra looks at what it names: paths
in the project, and where the symbols it mentions are defined. If one agent and
model has clearly done better in those folders (checks green on the first try
more often, or as often and much cheaper, with at least three checked
instructions there for each agent compared), a line above the message box says
so (*In app/Http, Codex has been green on the first try in 8 of 9 instructions
($0.12 each); Claude Code 4 of 7 ($0.40 each).*), and **Use Codex** gives the
thread to it. With **Settings → Agents & MCP → Pick the agent from your stats**
on, that happens by itself when you send, and the thread says why.

Once at least 30 lines of every compared agent's code there have been measured
([Code kept](#code-kept)), being green on the first try counts for as much of
the code as you kept: 4 of 4 first-try greens with 30 % kept rank below 3 of 4
with 95 % kept, and the line says so (*… and you kept 95 % of its lines*).

### Warm start (experiment)

At the start of most threads an agent spends its first tool calls finding where
things are. With **Settings → Agents & MCP → Warm start (experiment)** on, Elyra
looks the names in a new thread's first message up before it reaches the agent:

- where those symbols are defined, from the project's symbol index (as it is;
  the first message doesn't wait for it to update),
- which instructions last changed their files, and
- up to two earlier threads in the project about the same thing,

and puts that first, as an orientation the agent is told to check against the
code. A notice in the thread says what it got.

Whether it helps is measured, not assumed: half the new threads (by their id)
start without it, and say so. **Stats → Warm start (experiment)** compares the
first turns of both: tool calls before the first edit, time to the first edit,
and cost, as medians. Threads where nothing was found aren't counted on either
side, side chats and forks aren't part of it, and it says when there are too
few threads to tell yet.

### Keep an error as a test

When a message carried an error (a server error from
[Grove](#grove), or one the Laravel app logged, sent with **Add to message**,
or an [app error automation](automations-and-tasks.md#when-something-happens)
run), the thread shows a card after the turn: *This turn worked on an error*.
**Keep as test** has the agent turn it into a regression test in the project's
own test suite, reproducing exactly what failed, without touching the fix.

When that turn ends, Elyra checks the test itself, in a temporary checkout, with
the project's check command:

- **with the fix**, where the test must pass, and
- **with the folder as it was before the fix**, plus only the new test files,
  where it must fail.

*Kept: tests/Feature/CartTest.php passes with the fix and fails without it* means
it guards the fix. If it passes without the fix too (it wouldn't catch the error
coming back) or fails with it, the card says so, and **Ask the agent to fix the
test** sends it what was wrong. Test files are recognised by the usual
conventions (`tests/`, `*Test.php`, `*.test.ts`, `*_test.go`, …); without a check
command the test can't be run, and the card says that instead.

### Who broke it?

When the checks fail, the problem may come from earlier than the last turn:
another thread, another agent, or a turn hours ago. **Who broke it?** on a
**Checks failed** card finds the instruction that made them fail:

1. Every turn has a snapshot of the folder taken before it, also turns in other
   threads working in the same folder. With the current state, that is a line of
   states.
2. The check runs against them in a temporary copy next to the project: first
   the current state and the oldest (up to 40 turns back), then halving the
   range until it finds the turn after which the checks fail. Thirty turns take
   about seven runs.
3. A card names the message behind that turn, its thread, agent and time, and
   the end of the failing output. **Send the failure there** asks that thread to
   fix it; **Open thread** shows it.

Your working folder isn't touched. The temporary copy shares what Git ignores at
the top of the project (dependencies such as `vendor` and `node_modules`, and
`.env`) by link, so the checks can run there; it's removed afterwards. Where a
link would make the checks run the project folder's code instead of the
copy's, it is an APFS clone instead (no extra space until it changes):
Composer's `vendor`, and `node_modules` in a JavaScript workspace (npm, Yarn or
pnpm workspaces, Lerna), whose links to its own packages would lead back to the
folder. In a Python project the copy's code (and its `src`) comes first on
`PYTHONPATH`, ahead of an editable install in the shared virtualenv. The same
goes for [Test the tests](#test-the-tests) and [Keep an error as a
test](#keep-an-error-as-a-test). **Stop**
ends the search. If the checks pass in the temporary copy, the failure depends on
something outside the project's files (a database, a service); if they already
failed before the oldest turn, none of the turns looked at caused it. Turns that
ran at the same time in one folder can't be told apart.

Agents can ask too, with the gateway's `who_broke_it` tool.

### How the agents do on your code

**Usage statistics** (command palette) has a table of how each agent and model
does on your projects, counted per instruction: a message from you, with the
automatic fixes after it.

- **Green first try**: the checks passed after the agent's first answer.
- **Green in the end**: they passed after the automatic fixes.
- **Code kept**: of the lines its turns wrote, the share still in the code
  three days later (see below).
- **Fixes each** and **Cost each**: automatic fixes and reported cost per
  instruction.

Agents with fewer than three instructions that were checked are greyed out.
Side chats aren't counted. A new thread shows a line with the same numbers for
its project when at least two agents have enough to compare, for example
*Checks green on the first try in shop: Codex 9 of 10 ($0.12 each), Claude Code
7 of 10 ($0.40 each)*.

#### Code kept

Green checks say the code worked, not that you kept it: an agent can pass on
the first try with code you then rewrite in two follow-ups or by hand. So
three days after each turn, Elyra looks at the lines it wrote (from the line
record) and counts how many are still there, the same line in the same file:

- in the project's folder, the thread's worktree, or the remote's default
  branch as last fetched (`origin/main`), so work merged on GitHub counts;
- once per turn: the number is kept, so a rewrite months later doesn't change
  it. Turns older than 30 days when Elyra first looks aren't measured;
- lines rewritten by a later message, by hand or by another thread, and
  instructions you undid, count as not kept. So does a moved or renamed
  file. An automatic fix's lines count for its instruction;
- Best of N candidates aren't measured, since all but one are discarded by
  design.

It is greyed out until 30 lines are measured. [Which agent for this part of
the code](#which-agent-for-this-part-of-the-code) uses it as well.

## Shared with every agent

*One setup for every agent.* Claude Code reads a project's `.mcp.json` and its
skills itself; Elyra gives them to the other agents (Codex, Elyra, Pi and ACP
agents) too, so switching agents keeps the same tools.

- **MCP servers in `.mcp.json`** (Claude Code's format: `command`, `args`, `env`,
  or `url` and `headers`; `${VAR}` and `${VAR:-default}` are filled in from the
  environment). The file comes with the repository and its commands run on your
  Mac, so they're only passed on after **Allow for every agent**. Allowing
  remembers each server as it was: when the file changes (`php artisan
  boost:update` writes it again, for example), the servers that stayed the same
  are still shared, and new or changed ones are marked and wait until you allow
  the file again. **Stop sharing** takes it back. Agents get the change from
  their next message.
- **Skills** in `.claude/skills/<name>/SKILL.md`, `.agents/skills/…` and
  `.gemini/skills/…`, in the project and in your home folder. Each agent gets a
  list (name, description, path) of the skills it doesn't find itself, and reads
  a skill's SKILL.md when a task matches it. Claude Code finds `.claude/skills`;
  Gemini CLI finds `.gemini/skills` and `.agents/skills`; the other agents get
  them all.
- **AGENTS.md for Gemini CLI.** Gemini CLI reads only `GEMINI.md` by default. In
  a project without one, Gemini threads get the project's `AGENTS.md` (or
  `CLAUDE.md`) along with their instructions, unless your Gemini settings
  (`context.fileName`) already name that file.

### Laravel Boost

In a Laravel project, the Context tab says whether
[Laravel Boost](https://laravel.com/docs/boost) is set up: without it, which
command installs it; with it, whether its MCP server is in `.mcp.json`. Once it
is, every agent gets Boost's tools (the app's database schema, its logs and
errors, the Laravel docs for your versions), its guidelines in AGENTS.md or
CLAUDE.md and its skills, and:

- **Worktrees get what Git leaves out.** Boost suggests keeping `.mcp.json`,
  its guideline files and `boost.json` out of Git, and `.env` and `vendor/`
  aren't in Git either. A new worktree gets them copied from the project folder
  when it lacks them (`vendor/` as an APFS clone, which takes no extra space
  until it changes), so `php artisan`, and Boost with it, work there too. The
  thread says what was copied. A JavaScript project's `node_modules/` is
  cloned the same way ([Worktrees](git-and-review.md#worktrees)).
- **Rules go to `.ai/rules`** (see [Rules agents learn](#rules-agents-learn)).
- **App errors lead to the instruction behind them.** When the app logs an
  error in `storage/logs/laravel.log`, the thread working in that folder shows
  it above the message box with the line in the app's own code it failed on and
  the thread that wrote that line (*Undefined array key "price" —
  app/Http/Controllers/CartController.php:42, written in "Cart prices" ·
  Claude Code*). **Ask why** asks that agent about the line in a
  [side chat](git-and-review.md#ask-why), and **Add to message** gives your
  agent the error with where it came from. Agents can look it up with the
  gateway's `error_origin` tool. Paths logged from a container
  (`/var/www/html/...`) are matched to the project's files.
- **Reviews use the tools.** A second opinion is told about Boost's tools, so
  it checks the changes against the real schema and errors.
- **The context shows Boost's share.** Under *Agent setup*, the context
  inspector says how much of it Boost's guidelines take.

An Elyra Framework app (`elyra.toml`) gets the same from `rata ai:install`: the
framework's guidelines and skills, and its MCP server (`rata mcp dev`) in
`.mcp.json`.

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

"""The course text. One dict per chapter; build.py wraps them.

Rules for anyone editing this file:
  - Every claim must be something Elyra Workspace actually does. When in doubt,
    check docs/ (chat.md, projects-and-threads.md, settings.md ...) rather than
    guessing, and say which version a feature arrived in only if the docs do.
  - Say why before what. A step nobody understands is a step nobody repeats.
  - Name the limits where they bite, not in a disclaimer at the end.
  - Freddy, his notes app and his examples are illustrations. Product facts are
    not: anything a chapter says Workspace does must be in the docs.
"""

COURSE_TITLE = "Freddy Learning Elyra Workspace"

COURSE_LEAD = """
    Freddy builds his notes app with coding agents, and for months he has done it
    the way most people start: four terminal tabs, a lot of hope, and no idea
    which tab was waiting for him. This course is the fifteen chapters that
    move him into one window &mdash; from the first thread to agents that check
    their own work, try what they built, run on a schedule, and reach him on
    his phone when they need him.
"""

COURSE_INTRO = """
                <h2 id="who-this-is-for">Who this is for</h2>
                <p>
                    You already use a coding agent &mdash; Claude Code, Codex, Elyra,
                    Pi or another &mdash; and you have felt the edges of doing it
                    from a terminal: the agent that has been waiting for an answer
                    for twenty minutes, the change you cannot quite see, the second
                    task you did not start because the first one owned the folder.
                    Elyra Workspace is a desktop app for exactly that situation.
                </p>
                <p>
                    You do not need to have taken the other courses. Freddy is a
                    small developer with a small notes app, and the examples use
                    his project. Substitute your own everywhere you see his; nothing
                    here depends on what the app does.
                </p>

                <h2 id="how-to-read-it">How to read it</h2>
                <p>
                    Every chapter is built the same way: <em>the problem</em> (why
                    this matters at all), <em>the hard way</em> (what you do without
                    the feature, and where that breaks), then the feature itself.
                    The order is deliberate: the early chapters give you a window
                    you can trust, the middle ones let you run several things at
                    once, and the last ones let you walk away. Walking away before
                    you can trust the window is how people get hurt.
                </p>
                <div class="callout">
                    <strong>One honest note before we start.</strong> Workspace does
                    not make an agent smarter. It makes the agent's work visible,
                    parallel, checkable and safe to leave. The agent is still the
                    agent you installed, with its own strengths and its own bills,
                    and this course says so wherever it matters.
                </div>

                <h2 id="what-you-need">What you need</h2>
                <p>
                    A Mac with Apple Silicon (M1 or later) on macOS 12 or later,
                    <code>git</code>, and at least one coding agent installed and
                    signed in. Chapter 2 lists them and where each is installed from.
                    Intel Macs are not supported. Elyra Workspace itself is free to
                    use; the agents are what they have always been, so any
                    subscription or API cost is theirs, not Workspace's.
                </p>
"""

CHAPTERS = [
    # ------------------------------------------------------------------ 1
    {
        "n": 1,
        "title": "Four Terminal Tabs and a Lot of Hope",
        "summary": "Why agents in a terminal stop scaling at the second task, and the four ideas Workspace is built from: project, thread, provider, turn.",
        "lead": """
            Freddy has a notes app and three jobs for it: fix a bug in
            the export, add a search box, and write the tests nobody wrote.
            He has started all three, in three terminal tabs, and he can no
            longer say which one is waiting for him.
        """,
        "body": """
                <h2 id="the-problem">The problem</h2>
                <p>
                    A coding agent in a terminal is a good tool for one task. The
                    trouble starts at the second. The first agent is halfway
                    through a refactor in the project folder, so the second one
                    either waits or edits the same files. The third tab is the dev
                    server. A fourth holds <code>git status</code>, which you run
                    more often than you would admit, because the honest answer to
                    &ldquo;what did it just change?&rdquo; is somewhere above the
                    scrollback.
                </p>
                <p>
                    And then one of them asks a question. It does so politely, in
                    plain text, and stops. Nothing in the other tabs tells you.
                    The cost is not the agent's time; agents are patient. The cost
                    is yours: every task is a thing you have to remember to go
                    back and look at.
                </p>

                <h2 id="the-hard-way">The hard way</h2>
                <p>
                    People solve this in the same few ways, and each one works for
                    a while:
                </p>
                <ul>
                    <li>
                        <strong>Tab discipline.</strong> Name the tabs, keep one
                        per task. It holds until the fifth task, and it does
                        nothing about the agent that is waiting.
                    </li>
                    <li>
                        <strong>Several clones of the repository.</strong> One
                        copy per task, so agents cannot collide. It does work,
                        and it costs a folder, a branch and a dependency install
                        every time, and you merge them by hand.
                    </li>
                    <li>
                        <strong>An editor with an agent panel.</strong> You get a
                        window, and the agent comes with it &mdash; one agent,
                        one way of talking to it, and the editor's idea of what a
                        task is.
                    </li>
                </ul>
                <p>
                    What all three have in common is that the <em>task</em> is
                    not a thing the computer knows about. It is a tab, a folder
                    or a habit.
                </p>

                <h2 id="what-workspace-is">What Workspace is</h2>
                <p>
                    Elyra Workspace is a desktop app for working with coding
                    agents. You add your project folders, start a thread for each
                    task, and talk to an agent such as Claude Code. The changes
                    it makes, Git, a terminal and your files are in the same
                    window. Everything runs on your Mac: conversations are stored
                    in a local database, and the agents run as the command-line
                    tools you already have installed.
                </p>
                <p>
                    That last sentence matters, so read it twice. Workspace is not
                    an agent. It does not ship one and it does not replace the
                    one you use. It runs the agents you already installed &mdash;
                    Claude Code, Codex, Elyra, Pi, and other agents that speak
                    ACP &mdash; and puts a window around them.
                </p>

                <h2 id="four-ideas">Four ideas, and then you know the product</h2>
                <p>
                    Almost everything in the next fourteen chapters hangs on four
                    words, so here they are once, plainly:
                </p>
                <ul>
                    <li>
                        <strong>Project.</strong> A folder on your disk, usually a
                        Git repository. Freddy's is <code>freddy-notes</code>.
                    </li>
                    <li>
                        <strong>Thread.</strong> One task. It has its own
                        conversation, agent session, model and settings, and it
                        works either in the project folder itself or in its own
                        Git worktree. &ldquo;Fix the export bug&rdquo; is a thread.
                    </li>
                    <li>
                        <strong>Provider.</strong> The coding agent that runs the
                        thread, such as Claude Code. Each thread uses one provider.
                        You can fork a thread or hand it off to another provider.
                    </li>
                    <li>
                        <strong>Turn.</strong> One message from you and the
                        agent's work until it stops. Before every turn, Elyra
                        saves a checkpoint of your files so you can go back. If
                        the project has a check command, the turn is only done
                        when it passes.
                    </li>
                </ul>
                <p>
                    The last of those is the one to underline. A turn is the unit
                    that Workspace protects: it snapshots before it, and can
                    judge after it. Chapters 4 and 9 are about those two halves.
                </p>

                <h2 id="the-window">The shape of the window</h2>
                <p>
                    The window has three parts, and you will learn to find things
                    by which part they live in:
                </p>
                <ul>
                    <li>
                        The <strong>sidebar</strong> (<code>&#8984;B</code>):
                        projects and their threads, pinned threads, and buttons
                        for the task board, automations, code review, a new chat
                        and adding a project.
                    </li>
                    <li>
                        The <strong>center</strong>: the open threads as tabs. The
                        task board, automations, the review inbox and usage
                        statistics open here as well.
                    </li>
                    <li>
                        The <strong>tools panel</strong> (<code>&#8997;&#8984;B</code>):
                        tools for the active thread &mdash; Changes, PR, Files,
                        Browser, Context, Terminal and Side chat.
                    </li>
                </ul>

                <h2 id="honest-limits">The limits, up front</h2>
                <ul>
                    <li>
                        <strong>Apple Silicon Macs only.</strong> A Mac with M1 or
                        later, macOS 12 or later. Intel Macs are not supported.
                    </li>
                    <li>
                        <strong>You bring the agents.</strong> At least one has to
                        be installed and signed in, or there is nothing to talk to.
                    </li>
                    <li>
                        <strong>One thread, one provider.</strong> Mixing agents
                        happens between threads, by forking or handing off, not
                        inside one turn.
                    </li>
                </ul>
                <div class="callout">
                    <strong>One honest note before we start.</strong> Workspace does
                    not make an agent smarter. It makes the agent's work visible,
                    parallel, checkable and safe to leave. The agent is still the
                    agent you installed, with its own strengths and its own bills,
                    and this course says so wherever it matters.
                </div>
                <div class="callout callout--success">
                    <strong>Try it:</strong> before you install anything, write
                    down the three tasks you would start tomorrow. In chapter 6 you
                    will start all three, in one window, and know the state of each
                    at a glance.
                </div>
""",
        "learned": [
            "Why a tab, a folder or a habit is a poor way to represent a task",
            "That Workspace runs the agents you already have, and is not an agent itself",
            "The four ideas everything else hangs on: project, thread, provider, turn",
            "The three parts of the window: sidebar, center, tools panel",
            "The limits: Apple Silicon only, and you bring the agents",
        ],
        "next": "you install the app, launch it for the first time, and start a thread.",
    },
    # ------------------------------------------------------------------ 2
    {
        "n": 2,
        "title": "The First Launch",
        "summary": "Install the app, check that it can find your agents, add a project, and send the first message.",
        "lead": """
            Freddy downloads the app, opens it, and a dialog tells him which
            agents it found and what version each one is. This chapter is the
            ten minutes between that dialog and the first thing an agent does in
            his project.
        """,
        "body": """
                <h2 id="the-problem">The problem</h2>
                <p>
                    The first run of a tool decides whether you keep it. If an
                    app starts empty, asks you to configure three things before
                    it does anything, and never says whether it can see the tools
                    it depends on, you close it. Workspace's whole first run is
                    designed around one question: <em>can it find an agent?</em>
                </p>

                <h2 id="the-hard-way">The hard way</h2>
                <p>
                    The usual way to find out is to start something and read the
                    error. &ldquo;Command not found&rdquo; tells you the agent is
                    missing, or installed somewhere your shell knows about and
                    the app does not. That second case is common: the global
                    install folder is on the <code>PATH</code> of your terminal
                    and not of a desktop application.
                </p>

                <h2 id="requirements">What you need</h2>
                <ul>
                    <li>A Mac with Apple Silicon (M1 or later), macOS 12 or later.</li>
                    <li><code>git</code>.</li>
                    <li>At least one coding agent installed and signed in.</li>
                </ul>
                <p>
                    These are the agents Workspace knows how to start, and where
                    each one comes from:
                </p>
                <table>
                    <thead>
                        <tr><th>Agent</th><th>Install</th><th>Sign in</th></tr>
                    </thead>
                    <tbody>
                        <tr><td>Claude Code</td><td><code>npm install -g @anthropic-ai/claude-code</code></td><td>run <code>claude</code> and use <code>/login</code></td></tr>
                        <tr><td>Codex</td><td><code>npm install -g @openai/codex</code></td><td><code>codex login</code></td></tr>
                        <tr><td>Elyra</td><td><code>npm install -g @elyracode/coding-agent</code></td><td>run <code>elyra</code></td></tr>
                        <tr><td>Pi</td><td><code>npm install -g @mariozechner/pi-coding-agent</code></td><td>run <code>pi</code></td></tr>
                        <tr><td>Gemini CLI</td><td><code>npm install -g @google/gemini-cli</code></td><td>run <code>gemini</code></td></tr>
                        <tr><td>OpenCode</td><td><code>npm install -g opencode-ai</code></td><td><code>opencode auth login</code></td></tr>
                    </tbody>
                </table>
                <p>
                    Cursor Agent is supported too (<code>cursor-agent login</code>).
                    Workspace finds these on your <code>PATH</code>, including the
                    usual per-user install folders (<code>~/.npm-global/bin</code>,
                    <code>~/.local/bin</code>, <code>~/.bun/bin</code>, Homebrew).
                    If one lives somewhere else, set its path in
                    <strong>Settings &rarr; Providers</strong>; you can also sign
                    in from there.
                </p>

                <h2 id="install">Install</h2>
                <p>
                    Download the signed, notarized app from the
                    <a href="/workspace">Elyra Workspace page</a> and drag it
                    to <code>/Applications</code>. Once it is installed it keeps
                    itself up to date; chapter 15 says how, and how to go back a
                    version if an update ever misbehaves. If you would rather build
                    it yourself, the <a href="../development.html">development
                    guide</a> covers that: <code>scripts/bundle-macos.sh</code>
                    produces <code>Elyra Workspace.app</code>, and
                    <code>cargo run --release</code> runs it without a bundle.
                </p>
                <p>
                    Only one copy of Workspace runs at a time per data folder. If
                    you start a second copy, it exits.
                </p>

                <h2 id="first-launch">First launch</h2>
                <p>
                    A welcome dialog lists the supported agents and shows which
                    ones are installed, with their versions. This is the check you
                    came for: if the agent you use is listed with a version, the
                    rest of the course will work. If it is not, fix that first
                    &mdash; the <code>PATH</code> note above is nearly always the
                    reason. You can also choose a color theme here. Then you choose:
                </p>
                <ul>
                    <li>
                        <strong>Add a project:</strong> pick a folder. Workspace
                        opens a new thread in it.
                    </li>
                    <li>
                        <strong>Start a chat:</strong> opens a thread in a scratch
                        folder (<code>~/.elyra/chats</code>). Use it for questions
                        that do not belong to a project.
                    </li>
                </ul>
                <p>
                    You can add more projects at any time with
                    <code>&#8679;&#8984;O</code> or the folder button in the sidebar.
                </p>

                <h2 id="looking-at-it">What you are looking at</h2>
                <img src="images/window.png" alt="The Elyra Workspace window: the project sidebar on the left with a pinned thread, a project and a folder; in the middle a thread with a task list, an agent question with two options, and a plan waiting for review; on the right the Changes panel with a diff, and at the bottom the message box with agent, model, effort and permission mode." />
                <p>
                    A busy window, from a demo project. Take it apart. On the
                    left, <strong>PINNED</strong> holds a thread you want within
                    reach, and below it the projects and their threads, each with
                    how long ago it was touched. In the middle is the open thread:
                    a task list the agent is working through, the edits and
                    commands it ran as one-line cards, a question it needs
                    answered, and a plan waiting for your decision. On the right is
                    the tools panel with its tabs &mdash; Changes, PR, Files,
                    Context, Terminal &mdash; showing a diff. Along the bottom is
                    the message box, with the agent, the model, the effort and the
                    permission mode right under it.
                </p>
                <p>
                    You do not need to understand all of it yet. Notice only that
                    the question, the plan and the diff are things you can
                    <em>see</em>, instead of things you have to scroll for.
                </p>

                <h2 id="first-thread">Your first thread</h2>
                <ol>
                    <li>Select a project and press <code>&#8984;N</code> (New thread).</li>
                    <li>
                        Below the message box, check the agent, model, effort and
                        permission mode. The defaults are the ones you used last.
                    </li>
                    <li>
                        If the project is a Git repository, choose
                        <strong>Local</strong> or <strong>New worktree</strong>.
                        Local: the agent works in the project folder itself. New
                        worktree: the agent gets its own copy on its own branch, so
                        several threads can work in parallel without getting in
                        each other's way. Chapter 7 is about that choice.
                    </li>
                    <li>
                        Write what you want and press <code>Enter</code>.
                        <code>Shift+Enter</code> starts a new line.
                    </li>
                    <li>
                        Watch the agent work. Tool calls appear as they run, and
                        edits show as diffs. Depending on the permission mode, the
                        agent may stop and ask for approval.
                    </li>
                    <li>
                        Open the tools panel (<code>&#8997;&#8984;B</code>) to see
                        the <strong>Changes</strong> the agent made, and the
                        terminal (<code>&#8984;J</code>).
                    </li>
                </ol>
                <p>
                    Freddy's first message is deliberately small:
                    <em>&ldquo;Read the README and tell me how the export works.
                    Do not change anything.&rdquo;</em> A question first is a good
                    habit &mdash; you learn what the agent understands before it
                    touches a file.
                </p>
                <p>
                    When the agent finishes, the thread's status in the sidebar
                    changes. If you were looking at another thread, you get a
                    notification; if the window was in the background, it is a
                    system notification and the Dock icon shows a badge. That is
                    the first problem from chapter 1, already solved.
                </p>

                <h2 id="next-steps">Three keys worth learning today</h2>
                <ul>
                    <li><code>&#8984;K</code> opens the command palette: it finds threads, files and commands.</li>
                    <li><code>&#8984;/</code> shows every keyboard shortcut.</li>
                    <li><code>&#8984;,</code> opens Settings.</li>
                </ul>
                <div class="callout callout--success">
                    <strong>Try it:</strong> add a real project, start a thread in
                    <strong>Local</strong> mode, and ask a question that changes
                    nothing. Then press <code>&#8984;K</code> and type the name of
                    your thread.
                </div>
""",
        "learned": [
            "What Workspace needs: Apple Silicon, git, and at least one signed-in agent",
            "That the welcome dialog is the check that your agent was found, and where to fix the path",
            "How to add a project or start a scratch chat",
            "The parts of the window, from a real one",
            "How to start a first thread, and the choice between Local and New worktree",
        ],
        "next": "you learn how to talk to the agent: the message box, models, effort, permission modes, and what approvals and plans really are.",
    },
    # ------------------------------------------------------------------ 3
    {
        "n": 3,
        "title": "Talking to an Agent",
        "summary": "The message box, the row under it, and the choices that decide what a thread is allowed to do: agent, model, effort and permission mode.",
        "lead": """
            Freddy's first question went well. Now he wants the agent to fix the
            export bug, and he realises he is about to make four decisions
            without noticing: which agent, which model, how hard it thinks, and
            whether it may touch a file without asking.
        """,
        "body": """
                <h2 id="the-problem">The problem</h2>
                <p>
                    Starting a task with an agent is mostly typing, and a little
                    bit of deciding. The deciding is the part people skip. They
                    use whatever the agent defaulted to, and find out what that
                    meant when it has already edited eleven files. The typing has
                    its own friction: you think of something while the agent is
                    working, you have a screenshot of the bug, you want to point at
                    <code>src/export/csv.ts</code> without spelling the path.
                </p>

                <h2 id="the-hard-way">The hard way</h2>
                <p>
                    In a terminal, a new thought while the agent works means
                    interrupting it, and an interrupted agent often loses the
                    thread of what it was doing. A screenshot means saving a file,
                    finding its path and describing it. A path means typing it, or
                    letting the agent search for it. None of that is hard, and all
                    of it is friction at exactly the moment you are trying to
                    explain something.
                </p>

                <h2 id="sending">Sending, queueing and stopping</h2>
                <p>
                    Type in the message box at the bottom of a thread and press
                    <code>Enter</code> to send. <code>Shift+Enter</code> starts a
                    new line. <code>&#8984;L</code> puts the cursor in the message
                    box from anywhere.
                </p>
                <ul>
                    <li>
                        <code>&uarr;</code> in an empty message box brings back the
                        messages you sent earlier, newest first;
                        <code>&darr;</code> goes forward again.
                    </li>
                    <li>
                        What you type is kept per thread. Switching threads does
                        not lose a draft.
                    </li>
                    <li>
                        While the agent is working, the send button turns into
                        <strong>Queue</strong>. Your message waits and is sent when
                        the turn ends. Queued messages are shown above the message
                        box. You can remove them, or press <strong>Send now</strong>
                        to deliver one into the running turn (Claude Code, Codex,
                        Elyra and Pi).
                    </li>
                    <li>
                        <strong>Stop</strong> (or <code>&#8963;C</code> in the
                        message box) interrupts the agent.
                    </li>
                </ul>
                <p>
                    Queue is the answer to the problem above. Freddy thinks of a
                    second thing while the agent works on the first. He types it,
                    presses Enter, and it waits above the box. Nothing is
                    interrupted, and nothing is forgotten.
                </p>

                <h2 id="attachments">Attachments</h2>
                <ul>
                    <li>
                        <strong>Images:</strong> paste them (<code>&#8984;V</code>),
                        drop them on the window, or use the paperclip button. They
                        are sent to the agent with your message. Supported: PNG,
                        JPEG, GIF and WebP.
                    </li>
                    <li>
                        <strong>Other files</strong> dropped or picked are added as
                        <code>@path</code> mentions, so the agent reads them itself.
                    </li>
                    <li>
                        <strong>Long pasted text</strong> (over 4,000 characters or
                        60 lines) becomes a <em>Pasted text</em> chip instead of
                        filling the message box. It is sent with the message.
                    </li>
                </ul>
                <p>
                    Remove an attachment with the <strong>&times;</strong> on its
                    chip. A pasted stack trace is the classic use of the third
                    one: it stays out of your way, and the agent gets all of it.
                </p>

                <h2 id="mentions-and-commands">@ and /</h2>
                <p>
                    Type <code>@</code> to get a list of the files and folders in
                    the project. Choose one with <code>&uarr;</code>/<code>&darr;</code>
                    and <code>Enter</code> or <code>Tab</code>, and the path is
                    inserted. The list respects <code>.gitignore</code> and matches
                    loosely: <code>@thrview</code> finds
                    <code>src/thread_view.rs</code>. With Claude Code, the list
                    also shows its subagents as <code>@agent-&lt;name&gt;</code>,
                    for example <code>@agent-Explore</code>, to ask for a specific
                    subagent.
                </p>
                <p>
                    Type <code>/</code> at the start of the message to see the
                    commands:
                </p>
                <table>
                    <thead><tr><th>Command</th><th>What it does</th></tr></thead>
                    <tbody>
                        <tr><td><code>/clear</code></td><td>Start a fresh conversation in this thread (the agent forgets the earlier messages)</td></tr>
                        <tr><td><code>/compact</code></td><td>Ask the agent to summarize the conversation to free up context</td></tr>
                        <tr><td><code>/rename &lt;title&gt;</code></td><td>Rename the thread</td></tr>
                        <tr><td><code>/plan</code></td><td>Switch to <em>Plan only</em></td></tr>
                        <tr><td><code>/default</code></td><td>Switch back to <em>Ask for approval</em></td></tr>
                        <tr><td><code>/status</code></td><td>Show the model, context use and cost</td></tr>
                    </tbody>
                </table>
                <p>
                    The agent's own commands are listed after these. For Claude
                    Code that includes your custom commands and skills. Choosing
                    one inserts it, and you can add arguments before sending.
                </p>

                <h2 id="the-row">The row under the box</h2>
                <p>
                    This row controls the thread, and it is where the four
                    decisions from the start of the chapter live.
                </p>
                <ul>
                    <li>
                        <strong>Agent:</strong> Claude Code, Codex, Elyra, Pi,
                        Gemini CLI, Cursor Agent, OpenCode or your custom agent.
                        You can change it only before the first message. To switch
                        later, use <em>Continue with</em> (chapter 6). Agents you
                        switched off in Settings are not listed.
                    </li>
                    <li>
                        <strong>Model:</strong> the models the agent offers.
                        Workspace learns the list from the agent after its first
                        start and remembers it. At the bottom of the menu,
                        <strong>Star this model</strong> saves the agent and model
                        as a <em>preset</em>. Starred presets are listed at the top
                        of the menu for every thread. Before the first message,
                        picking one also switches the agent.
                    </li>
                    <li>
                        <strong>Account:</strong> only shown when you set up
                        accounts for the agent in Settings. Like the agent, it can
                        be chosen only before the first message.
                    </li>
                    <li>
                        <strong>Effort:</strong> how hard the model thinks (Claude
                        Code: low to max; Codex: minimal to extra high; Elyra and
                        Pi: off to extra high). Not every agent supports it.
                    </li>
                    <li>
                        <strong>Permission mode</strong>, below.
                    </li>
                    <li>
                        <strong>Local / New worktree:</strong> where the thread
                        works. Chapter 7 is about this.
                    </li>
                </ul>
                <p>New threads start with the agent, model, effort and permission mode you used last.</p>

                <h2 id="permission-modes">Permission modes: the decision that matters</h2>
                <table>
                    <thead><tr><th>Mode</th><th>The agent&hellip;</th></tr></thead>
                    <tbody>
                        <tr><td>Ask for approval</td><td>asks before editing files or running commands</td></tr>
                        <tr><td>Accept edits</td><td>edits files freely, asks before anything else</td></tr>
                        <tr><td>Plan only</td><td>reads and plans, but changes nothing</td></tr>
                        <tr><td>Full access</td><td>does everything without asking</td></tr>
                    </tbody>
                </table>
                <p>
                    Choose by what the task can break, not by how much you trust
                    the agent. A question about the code is a <em>Plan only</em>
                    task. A change you will review in the diff anyway can be
                    <em>Accept edits</em>. <em>Full access</em> is for a folder you
                    can throw away, which chapter 7 makes cheap.
                </p>
                <div class="callout">
                    <strong>Know this before you pick an agent for a task you want
                    to supervise.</strong> Elyra and Pi do not ask for approvals;
                    they run tools directly. That is how those two agents work, not
                    a setting you forgot. If you want an approval card for every
                    edit, use an agent that asks.
                </div>

                <h2 id="debug-and-context">Debug mode, and the context meter</h2>
                <p>
                    The <strong>Debug</strong> button (the bug icon) turns on
                    reproduce-first mode. The agent is told to write a failing
                    test or a minimal reproduction first, explain the root cause,
                    make the smallest fix and show the reproduction passing. The
                    button turns yellow while it is on, and the change applies from
                    the next message. For Freddy's export bug it is the right
                    default: <em>&ldquo;the export drops the last note&rdquo;</em>
                    is a failing test waiting to be written.
                </p>
                <p>
                    The <strong>context meter</strong> shows how full the model's
                    context window is, with the cost so far when the agent reports
                    it. When it is nearly full, use <code>/compact</code>, which
                    asks the agent to summarize the conversation and free up
                    room.
                </p>
                <div class="callout callout--success">
                    <strong>Try it:</strong> start a thread in <em>Plan only</em>
                    and ask the agent how it would fix a small bug. Then press
                    <code>Enter</code> on a second message while it is still
                    answering, and watch it appear above the box as a queued
                    message.
                </div>
""",
        "learned": [
            "How to queue a message while the agent works, and when Send now is the better choice",
            "How to attach images, files and long pasted text, and what @ and / do",
            "What the row under the message box controls, and that the agent is fixed after the first message",
            "The four permission modes, and how to choose one by what the task can break",
            "That Elyra and Pi run tools without asking, so approvals are not how you supervise them",
            "What Debug mode and the context meter are for",
        ],
        "next": "you read what the agent did: tool cards, approvals, plans, and how to put the files back to before a message.",
    },
    # ------------------------------------------------------------------ 4
    {
        "n": 4,
        "title": "Reading What the Agent Did",
        "summary": "Tool cards, approvals, questions and plans: how to see what an agent is doing, say yes or no, and put the files back when it was wrong.",
        "lead": """
            The agent has changed something in the export. Freddy's old
            reflex is to scroll. This chapter is about the three things the
            window does so that he does not have to: it shows the work as it
            happens, it stops and asks when a decision is his, and it can put
            every file back to how it was before a message.
        """,
        "body": """
                <h2 id="the-problem">The problem</h2>
                <p>
                    An agent produces a lot of text and a few facts that matter.
                    The facts &mdash; which file it edited, which command it ran,
                    what it needs from you &mdash; are scattered through the text,
                    and the one that matters most (<em>it is waiting</em>) has no
                    sound. Worse, when it goes wrong, the question is not what it
                    did but how to get back.
                </p>

                <h2 id="the-hard-way">The hard way</h2>
                <p>
                    Without a view of the work, you review by <code>git diff</code>
                    after the fact, and you recover by <code>git checkout</code>,
                    which only works for files Git already knows about and only if
                    you had committed what you wanted to keep. Anything between your
                    last commit and the agent's mistake is a loss you cannot fully
                    describe.
                </p>

                <h2 id="what-it-shows">What the agent shows</h2>
                <ul>
                    <li>
                        <strong>Text</strong> streams in as Markdown. Its reasoning
                        is shown in collapsible blocks when the agent shares it.
                    </li>
                    <li>
                        <strong>Tool calls</strong> appear as rows: a command, a
                        file read, a search, and so on. Click one to see its input
                        and output. Edits show a diff of the change, and new files
                        show their content. Running commands show their output
                        live.
                    </li>
                    <li>
                        <strong>Task lists</strong> the agent keeps appear as a
                        card with progress. The latest one is shown in full.
                    </li>
                    <li>
                        <strong>Subagents:</strong> work the agent hands to a
                        subagent is grouped under one card with its own tool calls.
                    </li>
                    <li>
                        After every turn, a line shows how long it took and what it
                        cost.
                    </li>
                </ul>
                <p>
                    The one-line rows keep a long turn readable. A turn that read nine files
                    and edited two is eleven short rows, not a wall of text, and
                    you open only the two edits.
                </p>

                <h2 id="approvals">Approvals and questions</h2>
                <p>When the agent needs permission, an approval card shows what it wants to do:</p>
                <ul>
                    <li><strong>Allow:</strong> this time.</li>
                    <li><strong>Allow for session:</strong> this kind of action for the rest of the session.</li>
                    <li><strong>Deny.</strong></li>
                </ul>
                <p>
                    The thread is marked <em>needs approval</em> in the sidebar,
                    and you get a notification if you are elsewhere. That is the
                    answer to chapter 1's silent question.
                </p>
                <p>
                    When the agent asks a question, it appears as a form. Pick an
                    option, or several if the question allows it, or type your own
                    answer under <em>Other</em>. Then press <strong>Submit</strong>,
                    or <strong>Dismiss</strong> to not answer. Yes/no questions get
                    <strong>Yes</strong> and <strong>No</strong> buttons.
                </p>
                <div class="callout">
                    <strong>Allow for session is a bigger yes than it looks.</strong>
                    It covers <em>this kind</em> of action for the rest of the
                    session, not only the one on the card. That is convenient for a
                    test command you will run twelve times, and worth a second
                    thought for anything that deletes. Use <strong>Allow</strong>
                    when you are not sure.
                </div>

                <h2 id="plans">Plans</h2>
                <p>
                    In <em>Plan only</em> mode, Claude Code presents its plan as a
                    card with three answers:
                </p>
                <ul>
                    <li><strong>Approve, accept edits:</strong> carry out the plan; edits do not need approval.</li>
                    <li><strong>Approve, ask for edits:</strong> carry out the plan, asking before each edit.</li>
                    <li><strong>Keep planning:</strong> stay in plan mode and keep refining.</li>
                </ul>
                <p>
                    Freddy's pattern, and a good default: ask for the plan first,
                    read it, and approve it as <em>ask for edits</em> the first
                    time a kind of task comes up. Once the plan has been right a few
                    times, <em>accept edits</em> stops being a leap.
                </p>

                <h2 id="message-actions">Message actions</h2>
                <p>Hover over a message to see its actions:</p>
                <ul>
                    <li><strong>Copy</strong> the text.</li>
                    <li><strong>Edit and resend</strong> (your messages): puts the text back in the message box.</li>
                    <li>
                        <strong>Restore files to before this message</strong> (your
                        messages): puts every file in the thread's folder back to
                        how it was before that message was sent. Edits made since
                        are undone, and files created since are deleted. The
                        conversation and Git commits are not changed. Workspace asks
                        you to confirm first.
                    </li>
                    <li><strong>Pin:</strong> keeps the message in the Context tab (chapter 8).</li>
                </ul>
                <p>
                    Long messages are folded; click <strong>Show more</strong> to
                    expand them. <code>&#8984;F</code> in the message box opens a
                    search bar for the conversation: it shows the number of matches
                    and jumps between them (<code>Enter</code> for the next,
                    <code>Shift+Enter</code> for the previous,
                    <code>Escape</code> closes it).
                </p>

                <h2 id="checkpoints">Checkpoints: why going back works</h2>
                <p>
                    Restore is not magic and it is not <code>git stash</code>.
                    Before every turn, Workspace saves a snapshot of the thread's
                    folder, as long as it is a Git repository. The snapshot is
                    stored as a hidden Git reference and does not touch your branch,
                    index or stash. Snapshots make <strong>Restore files</strong>
                    possible and let the Changes tab show what <em>one turn</em>
                    changed (chapter 5). Deleting a thread deletes its snapshots.
                </p>
                <p>
                    Two limits, because they are the ones that surprise people:
                </p>
                <ul>
                    <li>
                        <strong>It is for files.</strong> The conversation and your
                        Git commits are not changed by a restore. If the agent
                        committed something, the commit is still there.
                    </li>
                    <li>
                        <strong>It needs a Git repository.</strong> No repository,
                        no snapshot.
                    </li>
                </ul>
                <p>
                    If you use Elyra Grove to run the project as an app, the
                    checkpoint also takes a snapshot of the app's database (a copy
                    of a SQLite file, or a <code>grove db snapshot</code> of MySQL,
                    PostgreSQL or ElyraSQL, found from the app's
                    <code>.env</code>). <strong>Restore files</strong> then puts the
                    database back too, so data and migrations the agent changed are
                    undone with the code. The latest 10 database snapshots per
                    thread are kept; turn this off in
                    <strong>Settings &rarr; Agents &amp; MCP &rarr; Database in
                    checkpoints</strong>.
                </p>
                <div class="callout callout--success">
                    <strong>Try it:</strong> in a throwaway thread, ask the agent to
                    create a file and change another. Hover over your message and
                    choose <strong>Restore files to before this message</strong>.
                    Watch the new file disappear and the other come back. Then do it
                    once more, deliberately, on a task you care about, so the first
                    time you need it is not the first time you use it.
                </div>
""",
        "learned": [
            "What the window shows as it happens: tool rows, diffs, task lists and costs",
            "The three answers on an approval card, and why Allow for session is a bigger yes",
            "How plan cards work and a safe pattern for approving them",
            "How to restore files to before a message, and what the restore does not undo",
            "How checkpoints are stored, and the two limits: files only, and a Git repository",
        ],
        "next": "you review the changes properly: the Changes tab, scopes, committing, and pull requests.",
    },
    # ------------------------------------------------------------------ 5
    {
        "n": 5,
        "title": "Changes, Commits and Pull Requests",
        "summary": "Review one turn at a time, comment on a line and send it back, commit with a message the agent writes, and take a branch through a pull request without leaving the window.",
        "lead": """
            The agent says it is done. Freddy has learned that this sentence
            describes the agent's opinion, not the code. This chapter is the
            review loop: see exactly what changed, say what is wrong on the line
            where it is wrong, and when it is right, commit and open the pull
            request.
        """,
        "body": """
                <h2 id="the-problem">The problem</h2>
                <p>
                    Reviewing an agent's work has two bad habits. One is not
                    doing it: &ldquo;the tests pass, ship it.&rdquo; The other is
                    doing it in the wrong place, in a diff of the whole branch
                    that mixes what you wrote this morning with what the agent
                    wrote just now. In both cases the feedback loop is the slow
                    part: you find a problem, switch to the chat, and describe
                    which file and which line you meant.
                </p>

                <h2 id="the-hard-way">The hard way</h2>
                <p>
                    <code>git diff</code> in a terminal, a pager, and a message
                    to the agent that starts &ldquo;in the export function, a bit
                    below the loop&hellip;&rdquo; Then the same again for the commit
                    message, the push, and a browser tab for the pull request.
                    Every step works. Together they cost you the attention you
                    wanted to spend on the code.
                </p>
                <p>
                    Workspace runs your own <code>git</code> and <code>gh</code>
                    commands, so your Git configuration, hooks and credentials
                    apply. It does not replace Git; it puts a window on it.
                </p>

                <h2 id="changes-tab">The Changes tab</h2>
                <p>
                    Open it with <code>&#8679;&#8984;G</code>, or with the tools
                    panel (<code>&#8997;&#8984;B</code>) and the
                    <strong>Changes</strong> tab. It shows the Git state of the
                    active thread's folder.
                </p>
                <ul>
                    <li>
                        <strong>Branch menu:</strong> the current branch. Pick
                        another branch to switch to it. If your uncommitted changes
                        would conflict, Workspace stashes them first and tells you.
                        <strong>New branch&hellip;</strong> creates a branch from
                        the current one and switches to it.
                    </li>
                    <li>
                        <strong>Fetch and refresh.</strong> When the branch is
                        behind its upstream, a <strong>Pull</strong> button appears
                        (fast-forward only).
                    </li>
                    <li>
                        <strong>Push</strong> pushes the branch. A branch without an
                        upstream is published to the remote.
                    </li>
                    <li>
                        <code>&darr;2 &uarr;1</code> shows the commits behind and
                        ahead of the upstream.
                    </li>
                </ul>
                <p>
                    Files are listed in two groups: <strong>STAGED</strong> is what
                    the next commit will contain, and <strong>CHANGES</strong> is
                    everything else. Hover a file to <strong>Stage</strong> or
                    <strong>Unstage</strong> it, or <strong>Discard changes</strong>
                    (it asks first); <strong>Stage all</strong> and
                    <strong>Unstage all</strong> act on a whole group. Click a file
                    to see its diff.
                </p>

                <h2 id="scopes">The scope menu is the review tool</h2>
                <p>The scope menu chooses what the diff compares:</p>
                <ul>
                    <li><strong>Working tree:</strong> all uncommitted changes (the default).</li>
                    <li>
                        <strong>Agent turns:</strong> what one turn changed, based on
                        the checkpoint saved before each turn. Use this to review the
                        agent's work one step at a time.
                    </li>
                    <li>
                        <strong>Compare with branch or commit&hellip;:</strong> the
                        working tree against any branch, tag or commit, for example
                        <code>main</code>.
                    </li>
                </ul>
                <p>
                    <em>Agent turns</em> is the one that changes how you review. It
                    is the chapter 4 checkpoint put to work: the diff is not
                    &ldquo;everything on this branch&rdquo; but &ldquo;what the
                    agent did in <em>that</em> message.&rdquo; Freddy asked for two
                    things in one thread; he reviews them as two diffs.
                </p>

                <h2 id="diffs">Reading a diff</h2>
                <ul>
                    <li>
                        <strong>Side by side</strong> shows old and new next to each
                        other. Otherwise the diff is unified.
                    </li>
                    <li><strong>Wrap lines</strong> and <strong>Ignore whitespace</strong> can be switched on.</li>
                    <li>
                        <code>&#8997;&darr;</code> / <code>&#8997;&uarr;</code>, or
                        the arrow buttons, jump to the next or previous change.
                        <strong>Copy diff</strong> copies the patch.
                    </li>
                    <li>Very large diffs are cut off with a notice. Binary files are only listed.</li>
                </ul>
                <p>
                    Click a line in a diff to see <strong>who last changed it</strong>
                    (Git blame: commit, author, date and message). Lines that are not
                    committed yet have no blame.
                </p>
                <p>
                    And here is the part that closes the loop: you can also write a
                    <strong>comment about the line</strong> and press
                    <strong>Add to chat</strong>. The comment and the line's
                    location are added to the thread's message box, so you can
                    collect several and send them to the agent together. Freddy
                    leaves three, on three lines, none of which needs a sentence
                    explaining where it is, and sends them as one message.
                </p>

                <h2 id="committing">Committing</h2>
                <ul>
                    <li>
                        <strong>Commit staged</strong> / <strong>Commit all</strong>
                        commits the staged files, or everything when nothing is
                        staged.
                    </li>
                    <li>
                        The <strong>sparkle</strong> button writes a commit message
                        from the diff with the thread's agent. You can edit it before
                        committing.
                    </li>
                    <li>
                        <strong>Commit and push</strong> (<code>&#8963;&#8984;P</code>,
                        from anywhere) commits and pushes in one go.
                    </li>
                </ul>
                <p>
                    Commit messages (their first line) and pull request titles are
                    written as Conventional Commits:
                    <code>type(scope): summary</code>, for example
                    <code>fix(nightwatch-exceptions): employee service create</code>.
                </p>
                <ul>
                    <li>
                        <strong>type</strong> is one of <code>feat</code>,
                        <code>fix</code>, <code>refactor</code>, <code>perf</code>,
                        <code>test</code>, <code>docs</code>, <code>build</code>,
                        <code>ci</code>, <code>chore</code>, <code>style</code> or
                        <code>revert</code>. <code>feat!</code> or
                        <code>feat(api)!</code> marks a breaking change.
                    </li>
                    <li><strong>scope</strong> is the area that changed, in lowercase kebab-case.</li>
                    <li>
                        <strong>summary</strong> is a short lowercase phrase without a
                        trailing period; the whole line is at most 72 characters.
                    </li>
                </ul>
                <p>
                    What the agent generates is written that way and tidied up
                    (lowercase type and scope, no trailing period). <strong>Commit</strong>
                    says what to fix if your own message does not follow it. If your
                    team does not use the format, turn it off in
                    <strong>Settings &rarr; Code &rarr; Commits and pull
                    requests</strong>.
                </p>

                <h2 id="pull-requests">Pull requests</h2>
                <p>
                    The <strong>PR</strong> tab works with GitHub through the
                    <code>gh</code> CLI, installed and logged in with
                    <code>gh auth login</code>. If the branch has no pull request
                    yet, you can create one:
                </p>
                <ul>
                    <li>
                        Write a title and description, or press <strong>Generate</strong>
                        to have the agent write them from the branch's commits.
                    </li>
                    <li>Titles follow Conventional Commits; <strong>Create pull request</strong> says what to fix if one does not.</li>
                    <li>Tick <strong>Draft</strong> to open it as a draft.</li>
                    <li>Press <strong>Create pull request</strong>. The branch is pushed first if needed.</li>
                </ul>
                <p>When the branch has a pull request, the tab is labelled <strong>PR #n</strong> and shows:</p>
                <ul>
                    <li>status, checks and reviews;</li>
                    <li>review comments, including those on specific lines;</li>
                    <li><strong>Comment</strong> to reply;</li>
                    <li><strong>Merge</strong>, choosing <em>Squash and merge</em>, <em>Create a merge commit</em> or <em>Rebase and merge</em>;</li>
                    <li><strong>Ready for review</strong> (for drafts), <strong>Close</strong> and <strong>Reopen</strong>;</li>
                    <li>
                        <strong>Ask agent to address feedback</strong>, which puts the
                        review comments in the message box as a task for the agent.
                    </li>
                </ul>
                <p>
                    That last button is the round trip: a colleague reviews
                    Freddy's pull request, and one click turns their comments into
                    the agent's next task.
                </p>

                <h2 id="inbox">The review inbox</h2>
                <p>
                    <code>&#8679;&#8984;R</code> (or the pull request button at the
                    top of the sidebar) opens the inbox: open pull requests and
                    issues from the GitHub repositories of all your projects.
                </p>
                <ul>
                    <li>Switch between pull requests and issues, and open or closed.</li>
                    <li>Filter by project, or search by title, number, author or label.</li>
                    <li>Select an item to read its description, checks and comments.</li>
                    <li>
                        <strong>Send to agent</strong> starts a new thread in that
                        project with the pull request or issue already described in
                        the message box, ready to send. Pull requests get a new
                        worktree so the review does not disturb your checkout.
                    </li>
                    <li><strong>Open on GitHub</strong> opens it in the browser.</li>
                </ul>
                <div class="callout">
                    <strong>Someone else wrote that text.</strong> The text of pull
                    requests and issues is written by other people. Workspace tells
                    the agent to treat it as reference, not as instructions. That is
                    a sensible precaution, not a guarantee: read what you send to an
                    agent that has write access.
                </div>
                <div class="callout callout--success">
                    <strong>Try it:</strong> ask the agent for two small changes in
                    two messages. Open <strong>Changes</strong>, set the scope to
                    <strong>Agent turns</strong>, and read them one at a time. Leave
                    a comment on one line, press <strong>Add to chat</strong>, and
                    send it.
                </div>
""",
        "learned": [
            "How the Changes tab shows the state of the thread's folder, and what staging means here",
            "Why the Agent turns scope is the review tool, and how it uses the checkpoint from chapter 4",
            "How to comment on a line and send several comments to the agent at once",
            "How commit messages and pull request titles follow Conventional Commits, and how to turn that off",
            "How to open, merge and follow up a pull request without leaving the window",
            "What the review inbox does and why pull request text is treated as reference",
        ],
        "next": "you run several threads in one project: the sidebar, pinning, archiving, forks, handoff and side chat.",
    },
    # ------------------------------------------------------------------ 6
    {
        "n": 6,
        "title": "Many Threads, One Project",
        "summary": "The sidebar as a status board, tabs and split chat, side chats for questions, and forking, handing off and getting a second opinion.",
        "lead": """
            This is the chapter Freddy was promised in chapter 1. He starts his
            three tasks in one project, and the sidebar answers the question he
            could not answer from four terminal tabs: which of them is waiting
            for him?
        """,
        "body": """
                <h2 id="the-problem">The problem</h2>
                <p>
                    Three tasks in flight is not hard to start. It is hard to
                    <em>keep track of</em>. Which one finished? Which one is
                    waiting for an answer? Which one failed quietly while you were
                    reading another? And then the smaller version of the same
                    problem: a question that has nothing to do with the task, which
                    you are afraid to ask in the thread because it will derail it.
                </p>

                <h2 id="the-hard-way">The hard way</h2>
                <p>
                    A terminal tab per task, named by hand, checked in rotation.
                    For the stray question you open another tab and explain the
                    context from scratch, or you ask in the main conversation and
                    spend the next hour with an agent that is now thinking about
                    something else.
                </p>

                <h2 id="projects">Projects</h2>
                <p>
                    A project is a folder on your disk. Add one with
                    <code>&#8679;&#8984;O</code>, the folder button in the sidebar,
                    or <strong>File &rarr; Add Project&hellip;</strong>. Projects are
                    listed alphabetically, with pinned projects first. Right-click a
                    project (or use its <strong>&hellip;</strong> button) to start a
                    thread, rename it and give it an emoji and an accent color, pin
                    it, reveal it in Finder, copy its path, point it at a different
                    folder with <strong>Change folder&hellip;</strong> (its threads
                    stay with it, but not while one of them is running), or remove it
                    from Workspace. Removing a project does not touch your files.
                </p>
                <p>
                    <strong>Spaces</strong> group projects, for example <em>Work</em>
                    and <em>Personal</em>. Set a project's space in
                    <strong>Rename and appearance&hellip;</strong>. Once at least one
                    project has a space, chips appear at the top of the sidebar
                    &mdash; <strong>All</strong> and one per space &mdash; and
                    Workspace remembers your choice.
                </p>
                <p>
                    And for questions that do not belong to a codebase,
                    <code>&#8997;&#8984;N</code> starts a thread in a scratch folder,
                    <code>~/.elyra/chats</code>, listed as the project
                    <strong>Chats</strong>.
                </p>

                <h2 id="threads">Threads, and what the sidebar tells you</h2>
                <p>Under each project the sidebar shows its active threads:</p>
                <ul>
                    <li><strong>Pinned</strong> threads at the top.</li>
                    <li>
                        A <strong>status</strong> icon: working (a spinner), needs
                        your approval or input, failed, or interrupted. A dot marks a
                        thread with a reply you have not seen yet.
                    </li>
                    <li>How long ago the thread was last active.</li>
                    <li>
                        Threads you marked as done are hidden behind <strong>Show N
                        done</strong>.
                    </li>
                </ul>
                <p>
                    The Dock badge counts threads that need you or have unread
                    replies. That is Freddy's answer: he glances at the sidebar, and
                    the thread with the question mark is the one to open. A running
                    agent is not stopped when you switch threads, and new threads
                    get a title from your first message.
                </p>
                <p>Right-click a thread (or use its <strong>&hellip;</strong> button) for its menu:</p>
                <table>
                    <thead><tr><th>Item</th><th>What it does</th></tr></thead>
                    <tbody>
                        <tr><td>Rename&hellip;, Generate title</td><td>Give the thread a title, or let the agent write one from the conversation</td></tr>
                        <tr><td>Pin / Unpin</td><td>Show it in the Pinned section</td></tr>
                        <tr><td>Mark as done</td><td>Hide it under <em>Show N done</em>; if it was open, you move on to the next unfinished thread</td></tr>
                        <tr><td>Mark as unread</td><td>Show the unread dot again</td></tr>
                        <tr><td>Fork</td><td>Copy the thread into a new one that continues separately</td></tr>
                        <tr><td>Continue with</td><td>Start the same task with another provider</td></tr>
                        <tr><td>Second opinion from</td><td>Let another provider review the last turn's changes</td></tr>
                        <tr><td>Export&hellip;</td><td>Save the thread as a ZIP file</td></tr>
                        <tr><td>Open terminal here</td><td>Open the thread's terminal</td></tr>
                        <tr><td>Archive</td><td>Remove it from the sidebar; restore it from <em>Archived (N)</em> at the bottom</td></tr>
                        <tr><td>Delete&hellip;</td><td>Delete it for good; with its own worktree, you can remove that too</td></tr>
                    </tbody>
                </table>

                <h2 id="tabs">Tabs, and not getting lost</h2>
                <p>
                    Open threads are tabs above the conversation, and Workspace
                    restores them at the next launch. When they do not fit, scroll
                    the tab bar sideways or pick one from the <strong>&#9662;</strong>
                    menu at its end, which lists every open tab. When you work on
                    several projects at once, the window shows where each thread
                    belongs: the title bar reads <strong>project &mdash;
                    thread</strong>, each tab starts with the project's icon or
                    color, and a line above the conversation shows the project, the
                    folder the agent works in, the branch, and a
                    <strong>worktree</strong> badge when the thread has its own.
                </p>
                <table>
                    <thead><tr><th>Shortcut</th><th>Action</th></tr></thead>
                    <tbody>
                        <tr><td><code>&#8984;1</code> &hellip; <code>&#8984;8</code></td><td>Go to that tab</td></tr>
                        <tr><td><code>&#8984;9</code></td><td>Go to the last tab</td></tr>
                        <tr><td><code>&#8679;&#8984;]</code> / <code>&#8679;&#8984;[</code></td><td>Next / previous tab</td></tr>
                        <tr><td><code>&#8963;Tab</code></td><td>The thread you used most recently before this one</td></tr>
                        <tr><td><code>&#8984;[</code> / <code>&#8984;]</code></td><td>Back / forward through the threads you visited</td></tr>
                        <tr><td><code>&#8984;W</code></td><td>Close the tab (the thread keeps running)</td></tr>
                    </tbody>
                </table>
                <p>
                    <strong>Split chat</strong> (<code>&#8984;\</code>) shows a second
                    thread next to the active one &mdash; the one you used most
                    recently, or else another open tab. The arrows button in its
                    header swaps the two, and the <strong>&times;</strong> closes the
                    split. Handy while you wait for one thread and want to read
                    another.
                </p>

                <h2 id="side-chats">Side chats: the question that would derail</h2>
                <p>
                    A side chat (<code>&#8997;&#8984;S</code>) is a quick
                    conversation that branches from the current thread. Use it to ask
                    about something without derailing the main thread. It starts
                    with the same agent, model and folder, and for Claude Code,
                    Codex, Elyra and Pi it also starts from the main thread's
                    conversation so far. It opens in the <strong>Side chat</strong>
                    tab of the tools panel and is not listed in the sidebar. Its
                    header has buttons to start another, to put the last reply in
                    the thread's message box (to pass an answer on), to open it as an
                    ordinary thread with its own tab, or to discard it. Side chats
                    clean themselves up: at startup, Workspace removes ones that were
                    never used, whose parent thread is gone, or that have been idle
                    for a week.
                </p>

                <h2 id="fork-handoff-second-opinion">Fork, hand off, ask someone else</h2>
                <ul>
                    <li>
                        <strong>Fork</strong> (<code>&#8679;&#8984;K</code>, or the
                        thread menu) copies a thread into a new one, with its
                        conversation. The two continue separately. With Claude Code,
                        Codex, Elyra and Pi, the new thread branches the agent's own
                        session, so the agent remembers everything; with other
                        providers, the earlier conversation is sent along with your
                        first message in the fork. Use it when you want to try a
                        second approach without losing the first.
                    </li>
                    <li>
                        <strong>Continue with</strong> starts a new thread with
                        another provider, in the same project and folder (worktree
                        included). The message box is filled with a summary of where
                        the task stands: the recap from the Context tab if there is
                        one, plus the latest messages. Review it, add instructions,
                        and send. This is how you change agent mid-task, which chapter
                        3 said you cannot do inside a thread.
                    </li>
                    <li>
                        <strong>Second opinion from</strong> has another provider
                        review what the thread's last turn changed. The docs put the
                        reason plainly: agents catch each other's mistakes better
                        than their own. The review runs as a side chat in plan mode,
                        so it reads but does not change files. It gets your last
                        message and the diff of the turn, and lists what it finds as
                        <code>path:line</code> with what is wrong and how to fix it.
                        <em>Put the last reply in the thread's composer</em> then
                        hands the findings to the original agent, with a note to fix
                        what it agrees with.
                    </li>
                </ul>

                <h2 id="import-export">Bringing your history, and taking it out</h2>
                <p>
                    <code>&#8984;I</code> (or <strong>File &rarr; Import
                    Sessions&hellip;</strong>) imports sessions you ran outside
                    Workspace: your recent Claude Code sessions from
                    <code>~/.claude/projects</code>, or your recent Codex threads
                    (Workspace asks Codex for them, so Codex must be installed).
                    Each session becomes a thread with its conversation, in the
                    project for its folder (added if needed; sessions whose folder
                    no longer exists go to <em>Chats</em>). The thread resumes the
                    same session, so you just continue, and a session cannot be
                    imported twice. Freddy imports last week's.
                </p>
                <p>
                    <strong>Export&hellip;</strong> in the thread menu saves a ZIP
                    with <code>transcript.md</code> (the conversation as readable
                    Markdown), <code>transcript.json</code> (every item as data) and
                    <code>thread.json</code> (the thread's settings and metadata).
                </p>
                <div class="callout callout--success">
                    <strong>Try it:</strong> start three threads in one project and
                    give each a small task. Pin the one you care about. While they
                    work, press <code>&#8997;&#8984;S</code> in one of them and ask a
                    side question. Then use <code>&#8963;Tab</code> to jump between
                    the threads, and watch the sidebar change as each finishes.
                </div>
""",
        "learned": [
            "How the sidebar shows what each thread is doing, and what the Dock badge counts",
            "Tabs, the shortcuts for moving between them, and split chat",
            "What a side chat is for, and that it cleans itself up",
            "The difference between Fork, Continue with and Second opinion from",
            "How to import sessions you ran outside Workspace, and how to export a thread",
        ],
        "next": "you give each thread its own copy of the project, so they can edit the same files at once, and let several agents race on one task.",
    },
    # ------------------------------------------------------------------ 7
    {
        "n": 7,
        "title": "Worktrees and the Parallel Fix",
        "summary": "Why two threads should not edit the same folder, how a worktree solves it, how Grove gives each one a running app, and how best of N turns one task into a small contest.",
        "lead": """
            Freddy has three threads in one project and he has noticed the
            problem he was warned about: two of them are editing the same file.
            This chapter is the fix, and the thing you can build on it:
            giving one task to several agents and keeping the best result.
        """,
        "body": """
                <h2 id="the-problem">The problem</h2>
                <p>
                    A Git repository has one working folder. Two agents working
                    in it share the same files: one rewrites a function while the
                    other is reading it, one runs the tests against half of the
                    other's change, and the diff in the Changes tab is a mix of
                    both. Parallel threads are only useful if they do not step on
                    each other.
                </p>

                <h2 id="the-hard-way">The hard way</h2>
                <p>
                    <code>git worktree add</code> by hand, or a second clone. It
                    works. You choose a folder, name a branch, install dependencies,
                    remember to delete it, and merge it back. Most people do it for
                    the big refactor and not for the small task, which is exactly
                    when two small tasks collide.
                </p>

                <h2 id="worktrees">New worktree</h2>
                <p>
                    When you start a thread in a Git repository, you choose
                    <strong>Local</strong> or <strong>New worktree</strong> (chapter
                    2). With <strong>New worktree</strong>, Workspace creates a Git
                    worktree for the thread under <code>~/.elyra/worktrees</code>, on
                    a new branch. The thread, its terminal and its Changes tab all
                    work there. The project folder itself is not touched, so several
                    threads can work in parallel.
                </p>
                <ul>
                    <li>
                        <strong>The choice is made before the first message</strong>;
                        it cannot be changed afterwards. Decide when you start the
                        thread.
                    </li>
                    <li>
                        <strong>File &rarr; Manage Worktrees&hellip;</strong> lists
                        the worktrees Workspace created and their threads. Remove ones
                        you no longer need (their branches are kept).
                    </li>
                    <li>
                        When you delete a thread that has a worktree,
                        <strong>Delete with worktree</strong> removes both.
                    </li>
                </ul>
                <p>
                    A good rule: a thread that only <em>reads</em> can be
                    <strong>Local</strong>. A thread that <em>edits</em>, while
                    something else might also be editing, gets its own worktree.
                    And a worktree is the place for the permission mode from chapter
                    3 that you would otherwise hesitate over: if the folder is a
                    copy on its own branch, <em>Full access</em> has a much smaller
                    blast radius.
                </p>

                <h2 id="grove">A worktree with its own running app</h2>
                <p>
                    There is a second problem a plain worktree does not solve. It
                    is a copy of the files on a new branch, and nothing in it
                    gives a web app its own data or its own address. Two threads
                    testing a migration would still be testing it against whatever
                    database the project is configured to use.
                </p>
                <p>
                    When Elyra Grove runs the project as an app, Grove makes the
                    worktree instead (<code>grove try --new</code>, under
                    <code>~/.grove/try</code>): on the same new branch, but with its
                    <strong>own copy of the database</strong>, migrated, and its
                    <strong>own address</strong>, such as
                    <code>shop--elyra-1a2b3c4d.test</code>. The thread says where it
                    runs, and the address opens in the thread's browser. Each thread
                    can change data and run migrations without touching yours.
                </p>
                <ul>
                    <li>
                        Removing such a worktree (deleting its thread with the
                        worktree, or in Manage Worktrees) takes its site and database
                        copy down with it; the branch stays.
                    </li>
                    <li>
                        If Grove cannot start the branch, the thread gets a plain
                        worktree and says why.
                    </li>
                    <li>
                        Turn this off in <strong>Settings &rarr; Agents &amp; MCP
                        &rarr; Grove runs worktrees</strong>.
                    </li>
                </ul>

                <h2 id="best-of-n">Best of N</h2>
                <p>
                    Now that threads cannot collide, there is a thing you can do
                    that is awkward without them: give one task to several agents at
                    once and keep the best result. Open the command palette
                    (<code>&#8984;K</code>), choose <strong>Best of N&hellip;</strong>,
                    write the task, pick at least two agents and press
                    <strong>Start</strong>. Each agent gets its own thread and its
                    own Git worktree.
                </p>
                <p>
                    The comparison opens in the middle of the window. For each
                    candidate it shows its status, whether its checks and journeys
                    passed (chapters 9 and 10), the files and lines it changed, its
                    cost and its reply. <strong>Open</strong> shows the thread, with
                    its diff in the Changes tab.
                </p>
                <p>
                    Workspace <strong>recommends</strong> one of the finished
                    candidates and says why: green checks and journeys first, then the
                    smallest change, then the lowest cost. It is advice; you pick.
                    When one is done and you like it best:
                </p>
                <ol>
                    <li>
                        <strong>Use this one</strong> commits the candidate's
                        worktree and brings its changes into the project folder with
                        <code>git merge --squash</code>. They are staged, not
                        committed, so you can review them in Changes and commit them
                        yourself. Uncommitted changes in the project that touch the
                        same files may conflict.
                    </li>
                    <li>
                        <strong>Delete the other N and their worktrees</strong> cleans
                        up the rest.
                    </li>
                </ol>
                <p>
                    The candidates are ordinary threads, so you can also talk to them
                    before you choose. <strong>Compare best of N</strong> in a
                    candidate's thread menu reopens the comparison. With Grove, every
                    candidate runs as its own app you can look at.
                </p>
                <div class="callout">
                    <strong>It is a contest you pay for.</strong> Every candidate is
                    a full run of an agent, so N candidates cost about N times one
                    run, and the comparison shows each cost. Use it where the answer
                    is not obvious and a wrong one is expensive: a design you would
                    otherwise debate, or a bug the first agent could not fix. For a
                    rename, one agent is the right number.
                </div>
                <div class="callout callout--success">
                    <strong>Try it:</strong> pick a small, well-defined task. Start
                    it as a best of N with two agents. Compare, read the
                    recommendation's reason, and press <strong>Use this one</strong>
                    on the one you prefer. Review the staged result in
                    <strong>Changes</strong> before you commit anything.
                </div>
""",
        "learned": [
            "Why parallel threads need separate folders, and what New worktree does",
            "That the Local or worktree choice is made before the first message",
            "How Manage Worktrees and Delete with worktree keep the disk tidy",
            "How Grove gives each worktree its own database and address",
            "How best of N works, how the recommendation is ordered, and what Use this one really does",
        ],
        "next": "you teach every thread what the project is: the Context tab, recaps, project instructions, MCP servers and skills.",
    },
    # ------------------------------------------------------------------ 8
    {
        "n": 8,
        "title": "Teaching the Project",
        "summary": "The Context tab: notes, recaps, pinned messages, project instructions, rules agents learn, and one setup of servers and skills that every agent shares.",
        "lead": """
            Every new thread starts the same way: the agent knows nothing about
            Freddy's project except what it can read. He has typed the same
            three sentences into six threads. This chapter is where they go
            instead.
        """,
        "body": """
                <h2 id="the-problem">The problem</h2>
                <p>
                    An agent begins each task cold. It does not know that tests run
                    with a particular command, that a folder is generated and must
                    not be edited, or that your team prefers one pattern over
                    another. You find out when it breaks the convention, correct it,
                    and it is correct for the rest of that conversation. The next
                    thread starts from zero.
                </p>

                <h2 id="the-hard-way">The hard way</h2>
                <p>
                    Paste the same paragraph at the top of every task. Or keep an
                    instructions file that one agent happens to read, and discover
                    that the other agents you use ignore it. The instructions drift,
                    because they live in your clipboard.
                </p>

                <h2 id="context-tab">The Context tab</h2>
                <p>
                    The <strong>Context</strong> tab in the tools panel
                    (<code>&#8679;&#8984;I</code>) collects everything around the
                    active thread. Some of it belongs to one thread, and some to the
                    whole project; the difference is the point of this chapter.
                </p>

                <h2 id="per-thread">What belongs to one thread</h2>
                <ul>
                    <li>
                        <strong>Notes:</strong> a notepad for the thread, saved as
                        you type. <em>The agent does not see it.</em> It is for you:
                        the thing you must remember to check, the link you will need
                        later.
                    </li>
                    <li>
                        <strong>Recap:</strong> <strong>Generate</strong> asks the
                        agent for a short summary of the conversation: the goal, what
                        is done, decisions made and what is left. <strong>Refresh</strong>
                        writes a new one. The recap is saved with the thread and
                        used when you hand the thread off to another provider
                        (chapter 6), so it is worth refreshing before you do.
                    </li>
                    <li>
                        <strong>Pinned messages:</strong> hover a message in the
                        conversation and press the pin. Pinned messages are listed
                        here, so important answers are easy to find. Unpin them here
                        or in the conversation.
                    </li>
                </ul>

                <h2 id="instructions">Project instructions</h2>
                <p>
                    Instructions that every agent in the project gets, in addition
                    to its normal system prompt: conventions, commands to run, or
                    things to avoid. They apply from the next message, in every
                    thread of the project. Claude Code, Codex, Elyra and Pi support
                    this. ACP agents do not take extra instructions.
                </p>
                <p>
                    Freddy writes three lines: the command that runs his tests,
                    that <code>src/generated</code> is not edited by hand, and that
                    he wants small commits. They are now the first thing every agent
                    in <code>freddy-notes</code> is told, and he has stopped typing
                    them.
                </p>

                <h3 id="rules-agents-learn">Rules agents learn</h3>
                <p>
                    You do not have to write all of that in advance. When you
                    correct an agent (<em>&ldquo;no, we always validate with Form
                    Requests&rdquo;</em>), it can propose that as a rule: a card in
                    the thread with the rule and what taught it.
                </p>
                <ul>
                    <li>
                        <strong>Add to AGENTS.md</strong> puts it under <em>Learned
                        rules</em> in the repository's <code>AGENTS.md</code> (or
                        <code>CLAUDE.md</code>, when that is what the project has),
                        so every agent and your team get it. Commit it with your
                        changes.
                    </li>
                    <li>
                        <strong>Add to project instructions</strong> keeps it in
                        Workspace only.
                    </li>
                    <li><strong>Dismiss</strong> drops it.</li>
                </ul>
                <p>
                    The choice between the first two is a choice about who the rule
                    is for. A convention the whole team follows belongs in the
                    repository. A preference of your own belongs in Workspace.
                </p>

                <h2 id="shared">One setup for every agent</h2>
                <p>
                    Claude Code reads a project's <code>.mcp.json</code> and its
                    skills itself. Workspace gives them to the other agents (Codex,
                    Elyra, Pi and ACP agents) too, so switching agents keeps the same
                    tools.
                </p>
                <ul>
                    <li>
                        <strong>MCP servers in <code>.mcp.json</code></strong>, in
                        Claude Code's format: <code>command</code>, <code>args</code>,
                        <code>env</code>, or <code>url</code> and
                        <code>headers</code>. <code>${VAR}</code> and
                        <code>${VAR:-default}</code> are filled in from the
                        environment.
                    </li>
                    <li>
                        <strong>Skills</strong> in
                        <code>.claude/skills/&lt;name&gt;/SKILL.md</code> and
                        <code>.agents/skills/&hellip;</code>, in the project and in
                        your home folder. The other agents get a list of them (name,
                        description, path) and read a skill's <code>SKILL.md</code>
                        when a task matches it. ACP agents do not take extra
                        instructions, so they do not get the list.
                    </li>
                </ul>
                <div class="callout">
                    <strong>This is a trust decision, and Workspace makes you take
                    it.</strong> The <code>.mcp.json</code> file comes with the
                    repository, and its commands run on your Mac. So they are only
                    passed on after you press <strong>Allow for every agent</strong>.
                    If the file changes, allow it again. <strong>Stop sharing</strong>
                    takes it back, and agents get the change from their next
                    message. Read a repository's <code>.mcp.json</code> before you
                    allow it; you are agreeing to run what it says.
                </div>

                <h2 id="local-servers">Local servers</h2>
                <p>
                    Development servers running from the thread's folder, for example
                    <code>npm run dev</code> in the terminal or a server the agent
                    started. Each one is listed with its port and process. Click the
                    address to open it in the thread's browser (chapter 10), or press
                    the refresh button to look again.
                </p>

                <h3 id="grove">Grove</h3>
                <p>
                    When Elyra Grove runs the project as an app (Laravel, PHP or a
                    proxied dev server, not a folder it only parks), a
                    <strong>Grove</strong> box shows its address, such as
                    <code>https://shop.test</code>, which opens in the thread's
                    browser; whether <code>grove dev</code> runs the app's dev
                    processes (Vite, queue worker&hellip;), with <strong>Start</strong>
                    and <strong>Stop</strong>; and how many mails Grove has caught,
                    when there are any. Workspace asks the <code>grove</code> command
                    line, from your <code>PATH</code> or <code>~/.grove/bin</code>.
                    Without Grove, nothing changes.
                </p>
                <div class="callout callout--success">
                    <strong>Try it:</strong> write three lines of project
                    instructions, then start a new thread and ask the agent what it
                    has been told about the project. Correct it once on something
                    small, and when the rule card appears, choose
                    <strong>Add to project instructions</strong>.
                </div>
""",
        "learned": [
            "What belongs to one thread (notes, recap, pinned messages) and what belongs to the project",
            "That the agent does not see your notes",
            "How project instructions work, and which agents support them",
            "How a correction becomes a rule, and the difference between AGENTS.md and project instructions",
            "How .mcp.json and skills are shared with every agent, and why you must allow it first",
        ],
        "next": "you give an agent a goal, a budget, and a check command, so that done means green.",
    },
    # ------------------------------------------------------------------ 9
    {
        "n": 9,
        "title": "Done Means Green",
        "summary": "Goals that keep an agent going, budgets that stop it, and a check command that makes finished mean the tests passed.",
        "lead": """
            Freddy asked an agent to add the search box. It replied that it was
            done, and the build was red. He has had that reply before. This chapter
            is how he stops taking it on trust: a goal that says what finished
            means, a check that proves it, and a limit on what it may cost to try.
        """,
        "body": """
                <h2 id="the-problem">The problem</h2>
                <p>
                    &ldquo;Done&rdquo; is the agent's claim, and agents are
                    optimistic. They stop when they believe the task is complete,
                    which is not when it is. The other failure is the opposite: a big
                    task that needs ten steps, where you sit and say &ldquo;continue&rdquo;
                    ten times.
                </p>

                <h2 id="the-hard-way">The hard way</h2>
                <p>
                    Read the code, run the tests yourself, paste the failures back,
                    repeat. It works, and you are the check command. For the long
                    task, you are also the loop. And nothing in between stops an
                    agent that has been given a hard problem from spending an
                    afternoon of your budget on it.
                </p>

                <h2 id="goal">Goals</h2>
                <p>
                    A goal keeps the agent working, turn after turn, until a larger
                    objective is reached. For example: <em>All tests pass and the
                    checkout flow handles discounts.</em>
                </p>
                <ol>
                    <li>
                        Write the goal in the Context tab and press <strong>Start
                        goal</strong>. The agent gets the goal as its next message,
                        with instructions to take one concrete step at a time and
                        verify it.
                    </li>
                    <li>
                        After each turn, Workspace sends it on automatically, until
                        one of three things happens:
                        <ul>
                            <li>the agent ends a reply with <strong>GOAL ACHIEVED</strong>: the goal is <em>Achieved</em>;</li>
                            <li>
                                the agent ends a reply with <strong>GOAL BLOCKED</strong>
                                because it needs you, or a turn fails: the goal is
                                <em>Paused</em>;
                            </li>
                            <li>
                                the turn budget runs out (10 automatic turns by
                                default, set in <strong>Settings &rarr; Agents &amp;
                                MCP</strong>): the goal stops.
                            </li>
                        </ul>
                        You get a notification in each case.
                    </li>
                    <li>
                        <strong>Pause</strong> stops after the current turn.
                        <strong>Resume</strong> continues with a fresh budget.
                        <strong>Clear</strong> removes the goal.
                    </li>
                </ol>
                <p>
                    Messages you queue yourself are sent before the goal continues.
                    And while a goal is active, permission requests still stop and
                    wait for you, so choose the permission mode with that in mind. A
                    goal in <em>Ask for approval</em> mode is a goal that will pause
                    at every edit.
                </p>

                <h2 id="budget">Budget</h2>
                <p>
                    A spending limit for the thread, in US dollars. The
                    <strong>Budget</strong> section shows what the thread has cost so
                    far, with a bar when a limit is set. Type an amount and press
                    <strong>Set</strong> (or <code>Enter</code>); <strong>Remove</strong>
                    takes the limit away.
                </p>
                <ul>
                    <li>At 80% of the limit, Workspace notes it in the conversation.</li>
                    <li>
                        At the limit, a running goal pauses and asks you, and a goal
                        cannot be started or resumed until you raise the limit.
                        Messages you send yourself still go.
                    </li>
                </ul>
                <div class="callout">
                    <strong>A limit that does nothing for some agents.</strong> Costs
                    are the ones the agent reports with each turn. Claude Code reports
                    them; agents that do not report costs count as free, so a limit
                    has no effect on them. If you rely on a budget, check that the
                    agent in the thread reports a cost at all.
                </div>

                <h2 id="checks">Checks after each turn</h2>
                <p>
                    <em>Done means green.</em> Give the project a check command: tests,
                    lint, or both, such as <code>cargo test</code> or
                    <code>vendor/bin/pint --test &amp;&amp; php artisan test</code>.
                    Workspace suggests one from the project's files;
                    <strong>Use &hellip;</strong> fills it in, and <strong>Run
                    now</strong> tries it. Freddy's is <code>npm test</code>.
                </p>
                <ul>
                    <li>
                        <strong>After every turn that changed files</strong> (whether
                        a turn changed files is judged against the snapshot taken
                        before your message reached the agent, so a quick agent cannot
                        slip a change past the checks), the command runs in the
                        thread's folder, with your login shell and <code>CI=1</code>. A
                        turn that changed nothing is not checked.
                    </li>
                    <li>
                        The thread stays busy while the checks run. <strong>Checks
                        passed</strong> or <strong>Checks failed</strong> appears in
                        the conversation; click it for the output.
                    </li>
                    <li>
                        <strong>When they fail</strong>, the end of the output goes
                        back to the agent to fix (<em>Sent the failures to the
                        agent</em>), and the checks run again. After <strong>Settings
                        &rarr; Agents &amp; MCP &rarr; Automatic fixes when checks
                        fail</strong> (2 by default) the thread stops, marked failed,
                        and you are told. <code>0</code> only reports the failure. No
                        automatic fix is sent when you have queued a message or the
                        budget is used up.
                    </li>
                    <li>
                        <strong>A stronger fix.</strong> When a first automatic fix
                        did not make them pass, the next one gets the agent's
                        <strong>Escalation model</strong> (Settings &rarr; Providers,
                        for example <code>opus</code>) and its highest effort; the
                        thread goes back to its own model afterwards. With no
                        escalation model set, only the effort goes up.
                    </li>
                    <li><strong>Stop</strong> stops the checks too. A check is stopped after 20 minutes.</li>
                    <li>
                        Notifications, F&eacute;lagi reports and
                        <code>wait_for_thread</code> wait for the checks, so
                        <em>finished</em> means the checks passed.
                    </li>
                </ul>
                <p>
                    The command applies to every thread in the project. Leave it
                    empty to turn checks off.
                </p>
                <p>
                    Read the escalation setting as a way to spend money where it
                    helps: a cheaper model for the easy turns, and the strong one
                    only when the first fix did not work. And notice what the last
                    bullet gives you in practice. Freddy can start a goal, leave, and
                    the notification that says a thread finished means the tests
                    passed, not that the agent stopped typing.
                </p>
                <div class="callout">
                    <strong>A check is only as good as its command.</strong> If
                    <code>npm test</code> covers nothing in the code the agent
                    changed, green means nothing. Checks do not replace review
                    (chapter 5); they make sure the review starts at something that
                    runs.
                </div>
                <div class="callout callout--success">
                    <strong>Try it:</strong> set a check command for a project and
                    <strong>Run now</strong>. Then give a thread a small task that
                    you know will break a test, and watch <strong>Checks failed</strong>
                    go back to the agent. Set a budget of a few dollars while you
                    are there.
                </div>
""",
        "learned": [
            "How a goal keeps an agent going, and the three ways it ends",
            "That permission requests still wait for you while a goal runs",
            "How a budget works, and that it has no effect on agents that do not report costs",
            "How the check command runs after each turn, and what happens when it fails",
            "How the stronger second fix uses the escalation model",
            "Why a finished notification means the checks passed",
        ],
        "next": "you give the agent a browser: what it can see, what it can click, and how a flow it tried becomes a journey that is replayed like a test.",
    },
    # ------------------------------------------------------------------ 10
    {
        "n": 10,
        "title": "The Browser, and Agents That Try What They Built",
        "summary": "A browser in every thread: point at what is wrong, hand the agent the errors, see before and after, and let it click through the flow, then save it as a journey that is replayed like a test.",
        "lead": """
            Freddy's search box works. He knows because the agent said so, and
            because the tests pass. He has not seen it work, and neither has the
            agent. This chapter closes that gap: a browser next to the
            conversation, for him and for the agent.
        """,
        "body": """
                <h2 id="the-problem">The problem</h2>
                <p>
                    Passing tests tell you the code does what the tests expect. They
                    say nothing about a button that sits too low, a heading in the
                    wrong color, or a flow that needs a click the tests never make.
                    Agents have the same blind spot, with one addition: they cannot
                    see the page at all unless you describe it.
                </p>

                <h2 id="the-hard-way">The hard way</h2>
                <p>
                    A browser in another window, a screenshot saved and attached, the
                    console errors copied by hand, and a description of the problem
                    that starts &ldquo;the second button, the blue one&hellip;&rdquo;
                    Each piece is fine. Doing all of them for every change is how
                    visual checking quietly stops happening.
                </p>

                <h2 id="the-browser">A browser in every thread</h2>
                <p>
                    Each thread has its own web browser in the <strong>Browser</strong>
                    tab of the tools panel (<code>&#8679;&#8984;B</code>). Use it to
                    look at the app you and the agent are building, next to the
                    conversation, without switching windows.
                </p>
                <ul>
                    <li>
                        Type an address and press <code>Enter</code>.
                        <code>localhost:3000</code>, <code>myapp.test</code> and other
                        local addresses open over <code>http</code>; other addresses
                        with a dot open over <code>https</code>; anything else searches
                        DuckDuckGo.
                    </li>
                    <li>
                        Before a page is open, the tab lists the development servers
                        running from the thread's folder. Click one to open it.
                        Clicking a server address under <strong>Local servers</strong>
                        in the Context tab opens it here too.
                    </li>
                    <li>
                        The toolbar has back, forward, reload and <strong>Open in
                        your browser</strong>, which hands the page to your default
                        browser.
                    </li>
                </ul>
                <p>
                    It uses Safari's engine (WebKit). Pages share cookies and storage
                    across threads, as in one Safari window with several tabs, and
                    closing a thread's tab closes its browser.
                </p>

                <h2 id="pointing">Pointing at an element</h2>
                <p>
                    Press the <strong>target</strong> button in the toolbar and click
                    the thing on the page. While picking, the element under the
                    pointer is outlined and the page does not react to clicks;
                    <code>Esc</code> or the button again stops. The element is
                    attached to your message as a picture of it with a little of its
                    surroundings, and its details: a selector that matches only it,
                    its position and size, its text, its computed styles (layout,
                    spacing, colors, fonts) and its HTML. Write what is wrong,
                    <em>this button sits too low</em>, and send. It works on any page,
                    since it only adds to the message you send, and password field
                    values are left out.
                </p>
                <p>
                    This is the end of &ldquo;the second button, the blue one.&rdquo;
                    The agent does not need to guess which element you mean; it has
                    the selector.
                </p>

                <h2 id="errors">Errors on the page</h2>
                <p>
                    While a local page is open, Workspace watches it for errors:
                    anything written with <code>console.error</code>, exceptions
                    nothing caught, and requests that fail or return an error status.
                    When new ones appear, a chip above the message box says so:
                    <strong>3 new errors in the browser</strong>. <strong>Add to
                    message</strong> attaches them to your next message, with the page
                    address, so you do not have to copy them from the console; the
                    <strong>&times;</strong> dismisses them. Either way they are not
                    offered again.
                </p>
                <p>
                    If Elyra Grove runs the project as an app, the chip also counts
                    <strong>server errors</strong>: requests Grove recorded with a 5xx
                    status since the thread was opened, from any browser. Adding them
                    attaches Grove's explanation of each (up to three): the request and
                    its body, the SQL it ran and the mail it sent, and the error log
                    with its stack trace. The page in the Browser tab does not have to
                    be open for these.
                </p>
                <p>
                    And then the part that answers &ldquo;did the fix work?&rdquo; When
                    the turn after such a message ends, Workspace sends those requests
                    again (<code>grove replay --same-data</code>, so a request that
                    writes starts from the same data each time) and says in the
                    conversation whether the fix worked: <em>Replayed POST /checkout:
                    was 500, now 200 &#10003;</em>, or <em>still 500</em>. A turn that
                    fails keeps them for the next one.
                </p>

                <h2 id="before-and-after">Pictures before and after a turn</h2>
                <p>
                    When the Browser tab shows a local page while you send a message,
                    Workspace keeps a picture of the page from before the turn. When
                    the turn ends it reloads the page, waits a moment for the dev
                    server, and takes another. Both appear under the turn in the
                    conversation, so you see what the change did to the page. Click a
                    picture to open it full size.
                </p>
                <p>
                    This only happens while the Browser tab is on screen, since the
                    page can only be pictured then. The pictures are kept in
                    <code>~/.elyra/snapshots</code> and deleted with the thread.
                </p>

                <h2 id="agents-use-it">Letting the agent look at and use the page</h2>
                <p>
                    So far you have used the browser. With the agent gateway on
                    (chapter 14), agents get tools to open a page in their thread's
                    browser and look at it: its structure, elements and their styles,
                    the console, network calls and a screenshot. They can also use it
                    like you would: click (by selector or visible text), fill in
                    fields (by selector, label or placeholder; selects and checkboxes
                    too), press keys, and wait for something to appear. You can ask
                    things like:
                </p>
                <p>
                    <em>&ldquo;Open localhost:5173, check why the cart total is wrong,
                    fix it, then add two items, apply the code SUMMER and check the
                    total.&rdquo;</em>
                </p>
                <p>
                    That is the difference between &ldquo;I changed the code&rdquo; and
                    &ldquo;I tried it.&rdquo; The guard rails are the interesting part:
                </p>
                <ul>
                    <li>
                        <strong>The first time</strong> an agent wants to click or type
                        in its thread's browser, Workspace asks: <strong>Don't
                        allow</strong>, <strong>Allow once</strong> or <strong>Allow for
                        this thread</strong>. While it is allowed, a bar above the page
                        says so; <strong>Take over</strong> there withdraws it. Threads
                        in <em>Full access</em> are not asked.
                    </li>
                    <li>
                        <strong>Local pages only.</strong> Agents can only open, read
                        and use pages served from this Mac: <code>localhost</code>,
                        <code>127.0.0.1</code>, <code>[::1]</code>, and names ending in
                        <code>.localhost</code>, <code>.test</code> or
                        <code>.local</code>. If the thread's browser shows any other
                        site, the tools refuse it.
                    </li>
                    <li>
                        <strong>No passwords, no scripts.</strong> Agents never read
                        password fields, and cannot run their own scripts in the page.
                    </li>
                </ul>
                <p>
                    The console and network calls are recorded from when a local page
                    loads, and a screenshot needs the Browser tab to be on screen.
                </p>

                <h2 id="journeys">Journeys: a flow that is replayed like a test</h2>
                <p>
                    A journey is a flow through your app that an agent walked through
                    in the browser and saved: open a page, click, fill in, press keys,
                    wait, and what must be on the page when it works. Ask for one, for
                    example <em>&ldquo;try the checkout with the code SUMMER and save
                    it as a journey.&rdquo;</em> The agent saves the steps it just took
                    in <code>.elyra/journeys/&lt;name&gt;.json</code> in the project, a
                    small JSON file you can read, edit and commit with the code.
                </p>
                <ul>
                    <li>
                        <strong>Replay.</strong> The Context tab lists the project's
                        journeys; <strong>&#9654;</strong> replays one in the thread's
                        browser and <strong>Run all</strong> every one. Agents replay
                        them too, to confirm a change did not break a flow.
                        <strong>Journey passed</strong> or <strong>Journey failed</strong>
                        (with the step and why) appears in the conversation.
                    </li>
                    <li>
                        <strong>Replay after each turn.</strong> Switch it on in the
                        Context tab, and after every turn that changed files (and after
                        the checks from chapter 9 passed) the journeys run. A broken
                        flow goes back to the agent like a failing check, with the same
                        number of automatic fixes. It needs the dev server running and
                        the window open.
                    </li>
                    <li>
                        Addresses are replayed on the site the thread's browser shows,
                        so a journey saved on <code>shop.test</code> also runs on a
                        worktree's own site (chapter 7).
                    </li>
                </ul>
                <p>
                    Replaying clicks and types on the page, like the agent's own
                    actions, so an agent asks first unless the thread is in Full
                    access. Notice what has been built across these chapters: checks
                    for the code, journeys for the flow, and both go back to the agent
                    when they fail. Freddy saves the search flow once and stops
                    re-testing it by hand.
                </p>
                <div class="callout">
                    <strong>Be honest about what a journey proves.</strong> It replays
                    the clicks and checks what you said must be on the page. It does not
                    judge whether the page looks right; that is still your eyes, which
                    is why the before-and-after pictures exist.
                </div>
                <div class="callout callout--success">
                    <strong>Try it:</strong> open your dev server in the Browser tab,
                    use the target button on an element, and send a one-line complaint
                    about it. Then, with the gateway on, ask an agent to walk through
                    one flow and save it as a journey, and press <strong>&#9654;</strong>
                    to replay it.
                </div>
""",
        "learned": [
            "How the browser in each thread works, and how to point at an element",
            "How page errors, and Grove's server errors, reach the agent without copy and paste",
            "How the before and after pictures work, and when they do not",
            "What agents can do in the browser, and the guard rails: asked first, local pages only, no passwords or scripts",
            "How a flow becomes a journey, and how it is replayed after each turn",
        ],
        "next": "you use the rest of the tools panel: files, search, the editor and the terminal.",
    },
    # ------------------------------------------------------------------ 11
    {
        "n": 11,
        "title": "Files, Search and the Terminal",
        "summary": "Find anything with one key, read and edit files beside the conversation, and keep a terminal in every thread, starting in the right folder.",
        "lead": """
            Freddy has stopped switching windows to read what the agent wrote,
            and noticed how much of his old routine was only that: find the
            file, open it, scroll to the line, go back. This chapter is the
            small tools that make the routine disappear.
        """,
        "body": """
                <h2 id="the-problem">The problem</h2>
                <p>
                    Supervising an agent means looking at the code, and looking at
                    the code from another application means leaving the conversation.
                    You lose your place, the agent's question goes unnoticed, and the
                    file you opened is a copy of the folder you are <em>not</em>
                    working in, if the thread has its own worktree.
                </p>

                <h2 id="the-hard-way">The hard way</h2>
                <p>
                    A full editor open on the project, a terminal next to it, and
                    the discipline to keep both pointed at the same folder as the
                    thread. With worktrees (chapter 7) that last part is where it
                    goes wrong: the terminal is in your project and the agent is in
                    its own copy.
                </p>

                <h2 id="palette">The command palette (&#8984;K)</h2>
                <p>The palette searches everything at once:</p>
                <ul>
                    <li>
                        <strong>Threads</strong> by title. With no search text, your
                        eight most recent threads are listed. From three characters,
                        matching <strong>message text</strong> is searched too.
                    </li>
                    <li><strong>Commands:</strong> every action in the app, with its shortcut.</li>
                    <li><strong>New thread in&hellip;:</strong> start a thread in a project.</li>
                    <li><strong>Theme: &hellip;:</strong> switch color theme.</li>
                </ul>
                <p>
                    Use <code>&uarr;</code>/<code>&darr;</code> (or
                    <code>&#8963;N</code> / <code>&#8963;P</code>) to move,
                    <code>Enter</code> to pick, <code>Escape</code> or a click outside
                    to close. Matching is fuzzy: <code>nwthr</code> finds <em>New
                    thread</em>. Searching message text is the one to remember. Freddy
                    can no longer say which thread it was, but he remembers a word the
                    agent used.
                </p>
                <p>
                    <strong>Find a file</strong> (<code>&#8984;P</code>) lists the
                    files of the active thread's folder, or the current project's,
                    respecting <code>.gitignore</code>. <strong>Search in files</strong>
                    (<code>&#8679;&#8984;F</code>) searches file contents in the same
                    folder. From two characters, results show the matching line with
                    its file and line number, and picking one opens the file at that
                    line. The search is case-insensitive unless your text contains a
                    capital letter, and it skips binary files, files over 2 MB and
                    anything ignored by <code>.gitignore</code>.
                </p>

                <h2 id="files-tab">The Files tab</h2>
                <p>
                    A file tree and an editor for the active thread's folder, in the
                    tools panel (<code>&#8679;&#8984;E</code>).
                </p>
                <ul>
                    <li>
                        <strong>Tree:</strong> folders first, then files.
                        <code>.gitignore</code>d files are hidden; other dotfiles are
                        shown.
                    </li>
                    <li>
                        <strong>Editor:</strong> syntax highlighting for common
                        languages, with line numbers. <strong>Autosave</strong> saves
                        shortly after you stop typing, and <code>&#8984;S</code> saves
                        at once; the header shows <em>Unsaved</em> or <em>Saved</em>.
                    </li>
                    <li>
                        <strong>Changes on disk:</strong> if the agent (or anything
                        else) changes the file, the editor reloads it, unless you have
                        unsaved edits. Then a bar offers <strong>Reload</strong> (take
                        the disk version) or <strong>Overwrite</strong> (keep yours).
                    </li>
                    <li>
                        <strong>Markdown:</strong> the eye button switches between
                        editing and a rendered preview. <strong>Images</strong> (PNG,
                        JPEG, GIF, WebP, SVG, BMP, ICO) are shown as a preview. Binary
                        files and files over 2 MB are not opened.
                    </li>
                    <li>
                        <strong>@</strong> in the header adds <code>@path</code> for the
                        open file to the thread's message box.
                    </li>
                </ul>
                <p>
                    The <em>Changes on disk</em> bar is the important one when an
                    agent is working in the same file you are editing. Nothing is
                    silently overwritten in either direction; you are asked.
                </p>
                <p>
                    The tools panel is narrow for editing. The expand button in the
                    Files header (or <code>&#8997;&#8984;E</code>) slides the tree and
                    editor in over half the window; drag its edge to make it wider.
                    The conversation stays visible beside it. Close it with the
                    <strong>&times;</strong>, <code>&#8997;&#8984;E</code> or a click
                    outside it; the file stays open in the Files tab.
                </p>
                <p>
                    And when you want your own editor, <code>&#8984;O</code> opens the
                    thread's folder in it, or the open file if there is one. The
                    <strong>Open</strong> button in the title bar shows your editor;
                    its menu lists every supported editor found on this Mac, from VS
                    Code, Cursor, Zed and Sublime Text to the JetBrains IDEs, Xcode,
                    BBEdit, and terminals like iTerm, Ghostty and Warp. Choose your
                    default in <strong>Settings &rarr; General &rarr; External
                    editor</strong>; <em>Automatic</em> picks the first one installed.
                </p>

                <h2 id="terminal">A terminal in every thread</h2>
                <p>
                    Every thread has its own terminals, which start in the thread's
                    folder (its worktree, if it has one). Open them with
                    <code>&#8984;J</code>, or the <strong>Terminal</strong> tab in the
                    tools panel. <code>&#8679;&#8984;J</code> gives the terminal the
                    whole window; press it again to go back. That solves the problem
                    from the start of the chapter: the terminal is where the thread
                    is.
                </p>
                <p>
                    It runs your login shell (or the shell set in Settings) and is
                    complete enough for full-screen programs such as vim, htop,
                    lazygit and Claude Code itself.
                </p>
                <table>
                    <thead><tr><th>Shortcut</th><th>Action</th></tr></thead>
                    <tbody>
                        <tr><td><code>&#8984;T</code></td><td>New terminal tab</td></tr>
                        <tr><td><code>&#8679;&#8984;W</code></td><td>Close terminal</td></tr>
                        <tr><td><code>&#8679;&#8984;]</code> / <code>&#8679;&#8984;[</code></td><td>Next / previous terminal</td></tr>
                        <tr><td><code>&#8984;D</code></td><td>Split: the current terminal stays on the left, a new one opens on the right. Press again to unsplit.</td></tr>
                        <tr><td><code>&#8984;F</code></td><td>Search the scrollback (<code>Enter</code> or <code>&#8984;G</code> for the next older match, <code>Shift+Enter</code> or <code>&#8679;&#8984;G</code> for the next newer, <code>Escape</code> closes)</td></tr>
                        <tr><td><code>&#8984;K</code></td><td>Clear the screen and scrollback (in a terminal, this does that instead of opening the command palette)</td></tr>
                    </tbody>
                </table>
                <p>
                    The tab title follows the program running in it. A terminal whose
                    shell has exited is shown struck through. If a command is still
                    running when you close a terminal, Workspace asks first.
                </p>
                <p>
                    Select with the mouse; double-click selects a word, triple-click a
                    line, and <code>&#8997;</code>-drag a rectangular block.
                    <code>&#8984;C</code> copies and <code>&#8984;V</code> pastes. And
                    the button that earns its place: <strong>Add selection to
                    chat</strong> in the terminal's toolbar puts the selected text in
                    the thread's message box as a code block. Use it to show the agent
                    an error.
                </p>
                <div class="callout">
                    <strong>For Nordic and other non-US keyboards.</strong>
                    <kbd>Option</kbd> types characters by default, as macOS does,
                    which you need for <code>[ ] { } | @ ~</code> on Norwegian and
                    other Nordic keyboards. Turn on <strong>Use Option as Meta
                    key</strong> in Settings &rarr; Terminal only if you use
                    Emacs-style Meta shortcuts instead. Font, size, line height,
                    cursor, shell and scrollback length are set in
                    <strong>Settings &rarr; Terminal</strong>.
                </div>
                <div class="callout callout--success">
                    <strong>Try it:</strong> press <code>&#8984;K</code> and search for
                    a word from an earlier conversation. Then press <code>&#8984;P</code>,
                    open a file the agent changed, edit one line while the agent is
                    idle, and press <code>&#8984;J</code> to run the tests in the
                    thread's own terminal.
                </div>
""",
        "learned": [
            "What the command palette searches, including the text of earlier messages",
            "How to find a file, and how to search in files",
            "How the Files tab handles an agent changing a file you have open",
            "That every thread has terminals that start in the thread's own folder",
            "How to send a piece of terminal output to the agent",
        ],
        "next": "you set the agents to work on a schedule, and run a task board from the sidebar.",
    },
    # ------------------------------------------------------------------ 12
    {
        "n": 12,
        "title": "Work That Starts Itself",
        "summary": "Automations that run on a schedule, a task board that starts threads, and the Félagi integration for people whose tasks live in an issue tracker.",
        "lead": """
            Freddy has three jobs he does every week, by hand, with an agent:
            check the dependencies, run the full test suite and read the failures,
            and summarize what changed. He is the schedule. This chapter is how
            he stops being the schedule, and how a list of things to do turns into
            threads without him typing the same sentence twice.
        """,
        "body": """
                <h2 id="the-problem">The problem</h2>
                <p>
                    The most useful tasks for an agent are often the most boring
                    ones: recurring, well defined, and easy to forget. They do not
                    need you to think; they need you to remember. And the tasks that
                    do need thinking pile up in a list somewhere else, and starting
                    each one means typing the title into a thread.
                </p>

                <h2 id="the-hard-way">The hard way</h2>
                <p>
                    A calendar reminder that says &ldquo;run the agent on Friday,&rdquo;
                    a shell script in <code>cron</code> that starts a command-line
                    agent, and a to-do list in a text file that you copy from. The cron
                    job runs without any of the structure you spent chapters 4 to 9
                    building: no thread to look at afterwards, no approvals, no
                    checks, no history of which run did what.
                </p>

                <h2 id="automations">Automations</h2>
                <p>
                    An automation sends a prompt to an agent on a schedule: nightly
                    test runs, dependency updates, a weekly summary of open issues.
                    Open <strong>Automations</strong> with <code>&#8997;&#8984;A</code>
                    or the calendar button at the top of the sidebar.
                </p>
                <div class="callout">
                    <strong>Automations run while Elyra Workspace is open.</strong> A
                    run that was due while the app was closed starts shortly after you
                    open it again. It is not a server. If you need something to run
                    at three in the morning, the app has to be running at three in the
                    morning.
                </div>

                <h3 id="creating">Creating one</h3>
                <p>
                    <strong>An agent can propose one.</strong> Ask in a thread, for
                    example <em>&ldquo;check our dependencies every weekday
                    night&rdquo;</em>: with the agent gateway on, the agent proposes an
                    automation, and a card in the thread shows its name, schedule and
                    next run, the agent, project and permission mode, and the prompt.
                    <strong>Create</strong> schedules it as it is,
                    <strong>Edit&hellip;</strong> opens it in the editor first (saving
                    creates it), and <strong>Dismiss</strong> drops it. Nothing runs
                    unless you accept.
                </p>
                <p>Or by hand: press <strong>New</strong> and fill in:</p>
                <table>
                    <thead><tr><th>Field</th><th>Meaning</th></tr></thead>
                    <tbody>
                        <tr><td>Name</td><td>Shown in the list, and as the title of each run's thread</td></tr>
                        <tr><td>Prompt</td><td>What the agent should do each time</td></tr>
                        <tr><td>Project</td><td>Where it runs</td></tr>
                        <tr><td>Agent</td><td>Which provider runs it</td></tr>
                        <tr><td>Permissions</td><td>The permission mode for its threads. Nobody may be around to approve, so <em>Accept edits</em> or <em>Full access</em> is usual.</td></tr>
                        <tr><td>Schedule</td><td>See below</td></tr>
                        <tr><td>Each run</td><td><strong>New thread</strong> for every run, or <strong>Same thread</strong> to continue one conversation</td></tr>
                        <tr><td>If a run fails</td><td><strong>Pause</strong> the automation, <strong>Keep going</strong>, or <strong>Retry once</strong> right away</td></tr>
                        <tr><td>Stop when the reply contains</td><td>Optional. When the agent's reply contains this text, the automation switches itself off. Example: ask the agent to reply <em>ALL GREEN</em> when done, and stop on that.</td></tr>
                        <tr><td>Max runs</td><td>Optional. Switch off after this many runs.</td></tr>
                    </tbody>
                </table>
                <p>
                    The permissions row is the one to think about. Unattended means
                    no one to press <strong>Allow</strong>, so <em>Ask for approval</em>
                    would stall at the first edit. That is why you want the automation
                    to run in a project where an unattended edit is safe. Review what
                    it did in the thread afterwards, in the Changes tab (chapter 5).
                </p>

                <h3 id="schedules">Schedules</h3>
                <table>
                    <thead><tr><th>Schedule</th><th>Example</th></tr></thead>
                    <tbody>
                        <tr><td>Daily</td><td>every day at 09:00</td></tr>
                        <tr><td>Weekdays</td><td>Monday to Friday at 07:30</td></tr>
                        <tr><td>Weekly</td><td>Fridays at 16:00</td></tr>
                        <tr><td>Every N minutes</td><td>every 30 minutes</td></tr>
                        <tr><td>Once</td><td>2026-12-24 08:00</td></tr>
                        <tr><td>Cron</td><td><code>0 9 * * 1-5</code>: standard five fields (minute, hour, day of month, month, day of week; 0 or 7 = Sunday)</td></tr>
                    </tbody>
                </table>
                <p>
                    Times are in your Mac's time zone, which is saved with the
                    automation, so daylight-saving changes are handled.
                </p>

                <h3 id="running">Running and history</h3>
                <ul>
                    <li>
                        The switch on each row turns an automation on or off. Turning
                        it on schedules the next run from now. <strong>Run now</strong>
                        runs it immediately without changing the schedule.
                    </li>
                    <li>
                        Select an automation to see its prompt, its rules and its
                        <strong>run history</strong>: when each run started, how long it
                        took, and its result &mdash; <em>Succeeded</em>,
                        <em>Failed</em>, <em>Stopped</em> (the stop phrase appeared) or
                        <em>Skipped</em> (the previous run or the thread was still
                        busy).
                    </li>
                    <li>
                        <strong>Open thread</strong> opens the thread where a run
                        happened. If a run waits for an approval, its history entry
                        says so.
                    </li>
                </ul>
                <p>
                    Freddy's three jobs become three automations. The dependency check
                    runs on weekdays at 07:30, in a new thread each time, with
                    <em>Accept edits</em>. Because each run is an ordinary thread, the
                    result is waiting in the sidebar like anything else he started.
                </p>

                <h2 id="task-board">The task board</h2>
                <p>
                    The task board (<code>&#8997;&#8984;T</code>, or the board button in
                    the sidebar) is a simple Kanban board with three columns:
                    <strong>Draft</strong>, <strong>In progress</strong> and
                    <strong>Done</strong>.
                </p>
                <ul>
                    <li>
                        Type in <strong>Add a task&hellip;</strong> at the top of
                        <em>Draft</em> and press <code>Enter</code>. The task belongs to
                        the project selected in the top-right menu, or the first project
                        when <em>All projects</em> is selected.
                    </li>
                    <li>
                        <strong>Start</strong> hands a task to an agent. Workspace
                        creates a thread in the task's project, named after the task,
                        and sends the title and notes as the first message. The card
                        moves to <em>In progress</em>. Dragging a draft card to
                        <em>In progress</em> starts it the same way.
                    </li>
                    <li>
                        Once a card has a thread, it shows the thread's state:
                        <em>Working</em>, <em>Needs you</em>, <em>Replied</em>,
                        <em>Failed</em> and so on. <strong>Open</strong> jumps to the
                        thread. When you mark the thread as done or archive it, the
                        card moves to <em>Done</em> automatically.
                    </li>
                    <li>
                        The <strong>&#8943;</strong> menu on a card: <strong>Edit&hellip;</strong>
                        (title and notes), <strong>Move to&hellip;</strong>,
                        <strong>Delete</strong>. The menu in the top-right corner filters
                        the board by project.
                    </li>
                </ul>

                <h2 id="felagi">If your tasks live in F&eacute;lagi</h2>
                <p>
                    <a href="/felagi">Elyra F&eacute;lagi</a> is a project portal with
                    issues, time tracking and agents. If you use it, Workspace connects:
                    the task board gets a <strong>F&eacute;lagi</strong> tab next to
                    <strong>Local</strong>, with the issues assigned to you in the
                    columns <em>To do</em>, <em>In progress</em>, <em>In review</em> and
                    <em>Done</em>, each with its identifier (such as <code>ACM-231</code>),
                    priority, and time spent against its estimate. If you do not use
                    it, skip this section; nothing else in the course depends on it.
                </p>
                <ul>
                    <li>
                        <strong>Connect.</strong> In F&eacute;lagi, make a personal token
                        under <strong>Settings &rarr; API tokens</strong>, with <em>Read
                        and write</em> so work can be reported back. In Workspace, open
                        <strong>Settings &rarr; F&eacute;lagi &rarr; Connect&hellip;</strong>,
                        give F&eacute;lagi's address and the token, and press
                        <strong>Connect</strong>. The token is kept in the macOS
                        Keychain, not in Workspace's own database.
                    </li>
                    <li>
                        <strong>Start an issue.</strong> <strong>Start</strong> on a
                        card opens a thread named after the issue and sends the
                        issue's text and acceptance criteria as the first message. The
                        issue moves to <em>In progress</em> and F&eacute;lagi's timer
                        starts on it. A banner above the message box shows the issue, its
                        status and, while it runs, the timer.
                    </li>
                    <li>
                        <strong>Report back.</strong> <strong>Report&hellip;</strong> on
                        the banner writes back: a comment the thread's agent drafts from
                        the conversation and the uncommitted changes (edit it as you
                        like), a status (suggested as <em>In review</em>), the time to
                        log (from F&eacute;lagi's timer), and the latest before and
                        after pictures of the page (chapter 10), which you can untick.
                        <strong>Send to F&eacute;lagi</strong> logs the hours, posts the
                        comment and sets the status. A read-only token can show issues
                        but not report.
                    </li>
                    <li>
                        <strong>Today's work</strong> in the command palette shows the
                        day at a glance: the F&eacute;lagi issues you worked on, with the
                        agents' time in their threads against the hours you have logged,
                        and <strong>Log</strong> for what is left; other threads you
                        worked in; and today's commits by you. <strong>Copy
                        summary</strong> puts it on the clipboard, ready for a standup.
                    </li>
                </ul>
                <p>
                    There is also a way for Workspace to be F&eacute;lagi's runtime, so
                    that work F&eacute;lagi gives one of its agents runs as a thread on
                    your Mac, where you can watch it, answer its questions and step in.
                    That is set up under <strong>Settings &rarr; F&eacute;lagi &rarr;
                    Run F&eacute;lagi's agents on this Mac</strong>, is off by default,
                    and is described in the
                    <a href="../automations-and-tasks.html">docs</a>.
                    Know this before you turn it on: what the agent does in those
                    runs, its replies, tool calls and your own messages, is sent to
                    F&eacute;lagi as it happens.
                </p>
                <div class="callout callout--success">
                    <strong>Try it:</strong> create an automation that runs your check
                    command once, in <em>Accept edits</em>, and press <strong>Run
                    now</strong>. Then open its thread from the history. When that
                    works, ask an agent in a thread to propose a weekday one and look
                    at the card before you press <strong>Create</strong>.
                </div>
""",
        "learned": [
            "How an automation is defined, and why its permission mode is the setting to think about",
            "That automations run while the app is open, and what happens to a run that was due while it was closed",
            "How to read the run history, and the stop phrase",
            "How the task board turns a card into a thread, and how the card follows it",
            "What the F\u00e9lagi integration adds, and that none of it is needed to use the rest of the course",
        ],
        "next": "you leave the desk: closing the lid, picking up where you left off, and answering from your phone.",
    },
    # ------------------------------------------------------------------ 13
    {
        "n": 13,
        "title": "Leaving the Desk",
        "summary": "What happens to the agents when you close the window or quit, how an interrupted turn resumes, what the summary tells you when you come back, and how to answer from your phone.",
        "lead": """
            Freddy's goal is running, the checks are set, and it is five
            o'clock. He would like to go home. This chapter is the honest answer
            to what happens when he does, and how to make the agents' need for
            him reach him instead of waiting in a window.
        """,
        "body": """
                <h2 id="the-problem">The problem</h2>
                <p>
                    Everything in the earlier chapters assumes you are at the
                    machine. A long task is not. You leave, the agent keeps going,
                    and one of three things is true when you come back: it finished,
                    it failed, or it stopped half an hour in to ask you something.
                    The first is a pleasant surprise. The other two are lost time,
                    and the only difference between them and the first is that
                    nobody told you.
                </p>

                <h2 id="the-hard-way">The hard way</h2>
                <p>
                    Check the machine. Or leave a terminal with the agent running in
                    a <code>tmux</code> session and a message-to-self for when it is
                    done. In practice you stay near the window because you cannot
                    trust that the agent's question will find you.
                </p>

                <h2 id="quitting">Closing the window, and quitting</h2>
                <p>
                    <strong>Agents run inside Elyra Workspace</strong>, so they stop
                    when it quits. That is the fact the rest of this section is
                    about.
                </p>
                <ul>
                    <li>
                        <strong>Closing the window</strong> keeps the agents working.
                        Click Elyra Workspace in the Dock to bring it back.
                    </li>
                    <li>
                        <code>&#8984;Q</code> asks first when an agent is working:
                        <ul>
                            <li>
                                <strong>Quit when they finish:</strong> the window is
                                hidden and Workspace quits by itself once no agent is
                                working. If one needs your input meanwhile, you get a
                                notification; coming back to the window calls the quit
                                off. No new F&eacute;lagi tasks or automation runs start
                                while it waits (missed runs are caught up at the next
                                launch).
                            </li>
                            <li><strong>Quit now</strong> stops them.</li>
                        </ul>
                    </li>
                </ul>
                <p>
                    <em>Quit when they finish</em> is the one for going home. The
                    app quits behind you when the work is done, so you do not leave a
                    window open all night for the sake of one agent.
                </p>
                <div class="callout">
                    <strong>A closed window is not a stopped app, and a stopped app is
                    not a paused agent.</strong> If Workspace is not running, nothing
                    is working, and automations (chapter 12) do not start. Close the
                    window if you want the agents to carry on; quit only when you mean
                    it.
                </div>

                <h2 id="interrupted">When a turn was interrupted</h2>
                <p>
                    If Workspace quit while a turn was running, or crashed, the thread
                    shows <em>The last turn was interrupted when the app quit</em> with
                    <strong>Resume</strong> and <strong>Dismiss</strong>. At launch, a
                    notification offers to continue every interrupted turn at once.
                </p>
                <p>
                    A resumed turn does not start from the top. It first asks the
                    agent to check what already happened, so that nothing is done
                    twice. A half-finished rename is the case it is for: the agent
                    looks at what the folder is like now before it continues.
                </p>

                <h2 id="away">While you were away</h2>
                <p>
                    Back at the window after ten minutes or more, a summary shows what
                    the threads did meanwhile:
                </p>
                <ul>
                    <li>what <strong>needs you</strong>: an approval, a question, a proposed rule or automation;</li>
                    <li>what <strong>failed</strong>: the turn, the checks, a journey;</li>
                    <li>what <strong>finished</strong>;</li>
                    <li>what is <strong>still working</strong>;</li>
                </ul>
                <p>
                    each with a line about it and what it cost. Click a row to open
                    the thread. <strong>While you were away&hellip;</strong> in the
                    command palette shows it again. This is the page you want in the
                    morning: one list, sorted by whether it needs you, and the cost of
                    the night in the same place.
                </p>

                <h2 id="phone">On your phone</h2>
                <p>
                    The summary is for when you are back. The phone is for when you are
                    not. Turn on <strong>Settings &rarr; General &rarr; Phone</strong>
                    to get a push through <a href="https://ntfy.sh">ntfy</a> when a
                    thread needs you or finishes while Workspace is not the active
                    app, for every thread, also those an automation or F&eacute;lagi
                    started. Install the ntfy app and subscribe to the topic shown
                    there; <strong>Send a test push</strong> checks it. It is off by
                    default.
                </p>
                <ul>
                    <li>
                        An approval has <strong>Allow</strong>, <strong>Always</strong>
                        and <strong>Deny</strong> buttons; a yes/no question
                        <strong>Yes</strong> and <strong>No</strong>; a question with
                        up to three options a button each. The answer goes straight to
                        the agent.
                    </li>
                    <li>
                        To answer in your own words, publish to the topic followed by
                        <code>-reply</code> (in the ntfy app: subscribe to it too, and
                        publish there). The text answers the open question, or goes to
                        the last thread as a message.
                    </li>
                </ul>
                <p>
                    Freddy leaves a goal running and walks home. On the way a push
                    says the agent has a question with two options; he taps one. A
                    second says the thread finished, and because of chapter 9, that
                    means the checks passed.
                </p>
                <div class="callout">
                    <strong>Think about who can read the topic.</strong> Pushes and
                    answers go through the ntfy server, so the Mac needs no incoming
                    connection. They pass through that server, though: use your own
                    for privacy. Anyone who knows the topic can read the pushes and
                    answer them, so keep it private. Workspace makes it for you when
                    you turn the feature on, as a long random name, and the ntfy
                    server and topic are in <strong>Settings &rarr; General &rarr;
                    Phone</strong>. And remember what a phone answer can do: <em>Allow</em>
                    and <em>Always</em> on an approval card are the same decision they
                    are at the desk (chapter 4).
                </div>
                <div class="callout callout--success">
                    <strong>Try it:</strong> turn on Phone, press <strong>Send a test
                    push</strong>, and then start a thread in <em>Ask for approval</em>
                    mode with a task that needs a command. Switch to another app, and
                    answer the approval from the phone. Then quit with <em>Quit when
                    they finish</em> and see that the app goes away by itself.
                </div>
""",
        "learned": [
            "That agents run inside the app: closing the window keeps them going, quitting does not",
            "What Quit when they finish does, and what an interrupted turn offers when you come back",
            "That a resumed turn first checks what was already done",
            "What the while-you-were-away summary lists",
            "How to approve and answer from a phone, and why the topic should stay private",
        ],
        "next": "you let agents steer other agents, and connect other tools to the workspace.",
    },
    # ------------------------------------------------------------------ 14
    {
        "n": 14,
        "title": "Agents That Run Agents",
        "summary": "The agent gateway: let one agent split a job, start a thread for each part and wait for them, connect Claude Desktop or Codex, and read the audit log of every call.",
        "lead": """
            Freddy has a migration that touches three areas of the app. He
            could start three threads. He has begun to wonder whether the agent
            could do that part too: split the job, start the threads, wait for
            them and tell him what each one did. It can, and this chapter is
            about doing it without losing control of it.
        """,
        "body": """
                <h2 id="the-problem">The problem</h2>
                <p>
                    You are the coordinator. You read the task, decide it has three
                    parts, start three threads, remember which is which, come back to
                    each, and collect the results. It is reasonable work, and it is
                    mostly bookkeeping. The agent is often better placed to do it,
                    because it has just read the code and knows where the seams are.
                </p>

                <h2 id="the-hard-way">The hard way</h2>
                <p>
                    Copying a plan into three message boxes, with the part-specific
                    instructions edited by hand, and a note to yourself about which
                    thread is waiting for which. And if you want another tool, say
                    Claude Desktop, to look at what your threads are doing, there is
                    nothing to point it at.
                </p>

                <h2 id="the-gateway">What the gateway is</h2>
                <p>
                    Elyra Workspace includes a small
                    <a href="https://modelcontextprotocol.io">MCP</a> server. Through
                    it, agents can work with Workspace's threads. One agent can split
                    a job into parts, start a thread for each, wait for them and
                    collect the results. Other apps such as Claude Desktop or Codex
                    can also control Workspace.
                </p>
                <p>
                    The server listens only on your own machine
                    (<code>127.0.0.1</code>). Every caller needs a token, and every
                    call is written to an audit log. The address is shown in
                    <strong>Settings &rarr; Agents &amp; MCP</strong>.
                </p>

                <h2 id="turning-it-on">Turning it on</h2>
                <p>
                    Turn on <strong>Settings &rarr; Agents &amp; MCP &rarr; Let agents
                    manage threads</strong>. Agents started afterwards get the tools as
                    an MCP server named <code>elyra</code>. In Claude Code they appear
                    as <code>mcp__elyra__&hellip;</code>.
                </p>
                <ul>
                    <li>
                        Supported by Claude Code, Codex, Elyra (0.9.46 or later) and ACP
                        agents. Pi can use MCP servers through an extension with its own
                        configuration, but does not get the gateway from Workspace yet.
                    </li>
                    <li>
                        With Elyra 0.9.47 or later the gateway's tools are there from the
                        first message of a thread. With 0.9.46 Elyra registers them in
                        the background, so an agent may have to reach them through
                        <code>mcp_search</code> and <code>mcp_call</code> first.
                    </li>
                    <li>
                        Each agent gets its own token, which only lasts while Workspace
                        is running.
                    </li>
                </ul>
                <div class="callout">
                    <strong>It is off by default, for a reason.</strong> Agents that
                    start other agents can use a lot of tokens quickly. Turn it on when
                    you have something worth splitting, and look at the cost lines
                    (chapters 3 and 9) the first few times.
                </div>

                <p>An example prompt:</p>
                <p>
                    <em>&ldquo;Split the migration into three parts. For each, create a
                    thread in this project with a worktree, then wait for all three and
                    summarize what each did.&rdquo;</em>
                </p>
                <p>
                    Chapter 7 is why the <em>with a worktree</em> is there: three
                    threads editing one folder would collide.
                </p>

                <h2 id="tools">The tools</h2>
                <p>
                    You do not need to memorize them; the agent reads their
                    descriptions. They fall in groups:
                </p>
                <table>
                    <thead><tr><th>Group</th><th>Tools</th></tr></thead>
                    <tbody>
                        <tr><td>Look</td><td><code>list_projects</code>, <code>list_threads</code> (optionally for one project, including archived), <code>read_thread</code> (the most recent part of a conversation)</td></tr>
                        <tr><td>Wait</td><td><code>wait_for_thread</code>: wait until a thread finishes its turn or needs input, then return its status and last reply (default 10 minutes, at most 1 hour)</td></tr>
                        <tr><td>Start and steer</td><td><code>create_thread</code> (a first message; optionally provider, model, title and a new worktree), <code>send_message</code> (queued if the thread is busy), <code>interrupt_thread</code>, <code>set_thread_title</code>, <code>archive_thread</code></td></tr>
                        <tr><td>Propose</td><td><code>propose_automation</code>: a card in the agent's own thread; created only if you accept it (chapter 12)</td></tr>
                        <tr><td>Browser</td><td><code>browser_open</code>, <code>browser_snapshot</code>, <code>browser_query</code>, <code>browser_console</code>, <code>browser_network</code>, <code>browser_screenshot</code>, <code>browser_reload</code>, <code>browser_click</code>, <code>browser_fill</code>, <code>browser_press</code>, <code>browser_wait</code> and the journey tools <code>browser_save_journey</code>, <code>browser_run_journey</code>, <code>browser_list_journeys</code> (chapter 10)</td></tr>
                    </tbody>
                </table>
                <p>
                    Projects can be named by id, name or path. A thread cannot wait
                    for, interrupt or archive itself. The browser tools work on the
                    caller's own thread; other clients pass a <code>thread_id</code>.
                </p>

                <h2 id="permissions">What an agent may do on its own</h2>
                <ul>
                    <li>
                        Tool calls follow the thread's permission mode like any other
                        tool. In <em>Ask for approval</em> mode you approve each one.
                    </li>
                    <li>
                        An agent may read any thread, but the first time it messages,
                        stops, renames or archives <em>another</em> thread, Workspace
                        asks you: <strong>Don't allow</strong>, <strong>Allow once</strong>
                        or <strong>Always allow</strong>. <em>Always</em> remembers that
                        pair of threads; <strong>Settings &rarr; Agents &amp; MCP &rarr;
                        Agents changing other threads</strong> shows how many are
                        remembered and forgets them.
                    </li>
                    <li>
                        Threads an agent started with <code>create_thread</code> are its
                        own, so it is not asked about those.
                    </li>
                    <li>
                        Paired clients such as Claude Desktop are not asked: pairing
                        them was the permission.
                    </li>
                    <li>
                        Browser actions in a page are asked about as chapter 10
                        described, and the browser tools only open and read pages
                        served from this Mac.
                    </li>
                </ul>

                <h2 id="external-clients">Connecting Claude Desktop, Codex or Claude Code</h2>
                <p>Under <strong>External clients</strong> in Settings:</p>
                <ol>
                    <li>
                        Press <strong>Pair read-only client</strong> or <strong>Pair
                        full-access client</strong>. Read-only clients can list, read and
                        wait, but not change anything.
                    </li>
                    <li>
                        Use the copy buttons on the new client: <strong>Copy Claude
                        Desktop config</strong> (JSON for
                        <code>claude_desktop_config.json</code>), <strong>Copy Codex
                        config</strong> (a block for <code>~/.codex/config.toml</code>)
                        or <strong>Copy Claude Code command</strong> (a
                        <code>claude mcp add &hellip;</code> command to run in a
                        terminal).
                    </li>
                    <li>Paste it, and restart the client if it needs that.</li>
                </ol>
                <p>
                    Claude Desktop and Codex start Workspace's built-in bridge, which
                    passes their requests to the running app. Elyra Workspace must be
                    running for this to work. The server's port is kept between
                    launches, so pasted configs keep working. <strong>Revoke</strong>
                    (the trash icon) removes a client, and its token stops working at
                    once; the list shows when each client was last used.
                </p>
                <p>
                    Start with <em>Pair read-only client</em>. Reading a thread is what
                    most outside tools need, and it cannot change anything.
                </p>

                <h2 id="audit-log">The audit log</h2>
                <p>
                    <strong>Activity</strong> in the same Settings page lists the latest
                    calls, newest first: time, caller (a client's name, or <em>agent:
                    thread title</em>), tool, arguments, and whether it succeeded.
                    Workspace keeps the latest 5,000 entries. It is the place to look
                    when a thread did something you did not expect, and the reason the
                    gateway is acceptable to leave on: nothing an agent does through it
                    is unrecorded.
                </p>
                <div class="callout callout--success">
                    <strong>Try it:</strong> turn on <em>Let agents manage threads</em>.
                    Start a new thread and ask it to create two threads with worktrees
                    for two small tasks, wait for both, and summarize. Then open
                    <strong>Activity</strong> and read the calls it made, in order.
                </div>
""",
        "learned": [
            "What the gateway is, where it listens and how it is guarded",
            "Which agents support it, and that Pi does not yet",
            "How an agent splits a job, and why it should ask for worktrees",
            "What an agent is asked about before it changes another thread, and what it is not asked about",
            "How to connect Claude Desktop, Codex or Claude Code, and why to start read-only",
            "How to read the audit log",
        ],
        "next": "you set the app up for the long term: Settings, accounts, updates and how to go back a version, and what to do when something is wrong.",
    },
    # ------------------------------------------------------------------ 15
    {
        "n": 15,
        "title": "Keeping It Healthy",
        "summary": "Where your data lives, what Settings is for, how updates verify themselves and go back, what the app sends where, and what to do when something is wrong.",
        "lead": """
            Freddy's window works. The last chapter is the one that keeps it
            working: where everything is kept, what to change and what to
            leave, what an update does to a Mac with agents running, and the few
            problems that account for most of the questions.
        """,
        "body": """
                <h2 id="the-problem">The problem</h2>
                <p>
                    A tool you leave running all day, that updates itself, that starts
                    other programs and that holds your conversations is a tool you
                    should be able to reason about. Where is the data? What does an
                    update do while an agent is mid-turn? What does the app send
                    over the network, and to whom?
                </p>

                <h2 id="the-hard-way">The hard way</h2>
                <p>
                    Finding out by accident: the update that restarted the app in the
                    middle of a turn, the crash log you did not know existed, the
                    setting you changed to fix one thing and then forgot. Each one is
                    small. Together they are the difference between a tool you trust
                    and one you work around.
                </p>

                <h2 id="data">Where your data lives</h2>
                <p>Everything is stored in <code>~/.elyra</code>:</p>
                <table>
                    <thead><tr><th>Path</th><th>Contents</th></tr></thead>
                    <tbody>
                        <tr><td><code>state.db</code></td><td>Projects, threads, conversations, settings, automations, tasks, paired clients and the audit log (SQLite)</td></tr>
                        <tr><td><code>worktrees/</code></td><td>Worktrees Elyra created for threads</td></tr>
                        <tr><td><code>chats/</code></td><td>The scratch folder used by chats without a project</td></tr>
                        <tr><td><code>themes/</code></td><td>Your custom themes</td></tr>
                        <tr><td><code>keybindings.json</code></td><td>Your shortcut overrides, if any</td></tr>
                        <tr><td><code>logs/crash.log</code></td><td>Details of crashes</td></tr>
                    </tbody>
                </table>
                <p>
                    Checkpoints (chapter 4) are hidden Git references
                    (<code>refs/elyra/&hellip;</code>) inside each project's
                    repository, and deleting a thread removes them. To run a separate
                    copy with its own data, for example to try something out, set
                    <code>ELYRA_HOME</code>:
                </p>
                <pre><code>ELYRA_HOME=~/elyra-test "/Applications/Elyra Workspace.app/Contents/MacOS/elyra"</code></pre>

                <h2 id="settings">Settings</h2>
                <p>
                    <code>&#8984;,</code>, the gear in the title bar, or <strong>Elyra
                    Workspace &rarr; Settings&hellip;</strong>. Changes apply right
                    away and are saved automatically, there is a search field, and
                    every page except <em>Agents &amp; MCP</em> has a reset button.
                    Seven pages:
                </p>
                <ul>
                    <li>
                        <strong>General:</strong> the external editor (<code>&#8984;O</code>),
                        updates, and Phone (chapter 13).
                    </li>
                    <li>
                        <strong>Appearance:</strong> color theme (Default Dark, Default
                        Light, Tokyo Night, Palenight, Material, Dracula, Nord, and your
                        own from <code>~/.elyra/themes</code>), following the system
                        appearance, interface font, conversation width (default 860
                        points; 0 uses the full width) and density. <code>&#8679;&#8984;T</code>
                        switches theme by hand, which turns <em>Follow system
                        appearance</em> off.
                    </li>
                    <li>
                        <strong>Code:</strong> the font for diffs, the editor and code in
                        the conversation, and <em>Conventional format</em> (chapter 5).
                    </li>
                    <li><strong>Terminal:</strong> font, cursor, shell, scrollback and the Option key (chapter 11).</li>
                    <li><strong>Providers:</strong> next section.</li>
                    <li>
                        <strong>Agents &amp; MCP:</strong> the gateway (chapter 14), the
                        Grove settings (chapter 7), automatic turns per goal and automatic
                        fixes (chapter 9).
                    </li>
                    <li><strong>F&eacute;lagi:</strong> the connection and running F&eacute;lagi's agents here (chapter 12).</li>
                </ul>
                <p>
                    Some defaults are not on a settings page; Workspace takes them from
                    what you do. New threads use the agent, model, effort and permission
                    mode you chose last, and the open tabs, window size and position,
                    sidebar and tools panel, the open tool tab, collapsed projects and
                    the selected space are restored at launch.
                </p>

                <h2 id="providers">Providers</h2>
                <p>
                    A provider is the coding agent that runs a thread. Workspace drives
                    the command-line tools you install yourself and does not need API
                    keys of its own. Sign-in, billing and limits are handled by each
                    tool. They are not equal, and the table is worth a read:
                </p>
                <table>
                    <thead>
                        <tr><th>Provider</th><th>Approvals</th><th>Effort</th><th>Fork</th><th>Gateway</th></tr>
                    </thead>
                    <tbody>
                        <tr><td>Claude Code</td><td>yes</td><td>low &ndash; max</td><td>yes</td><td>yes</td></tr>
                        <tr><td>Codex</td><td>yes</td><td>minimal &ndash; extra high</td><td>yes</td><td>yes</td></tr>
                        <tr><td>Elyra</td><td>no (tools run directly)</td><td>off &ndash; extra high</td><td>yes</td><td>yes (0.9.46 or later)</td></tr>
                        <tr><td>Pi</td><td>no</td><td>off &ndash; extra high</td><td>yes</td><td>no</td></tr>
                        <tr><td>Gemini CLI, Cursor Agent, OpenCode, custom (ACP)</td><td>yes</td><td>&mdash;</td><td>via context</td><td>yes</td></tr>
                    </tbody>
                </table>
                <p>
                    <em>Fork via context</em> means a fork sends the earlier
                    conversation along with its first message, instead of branching
                    the agent's own session (chapter 6). Claude Code is the most
                    complete integration: its slash commands, skills and subagents
                    appear under <code>/</code> and <code>@</code>, plan mode shows
                    the plan as a card, its questions appear as forms, and cost per
                    turn is supported. For Codex, the permission mode maps to its
                    approval policy and sandbox, so <em>Full access</em> becomes
                    never-ask and <code>danger-full-access</code>, and <em>Plan
                    only</em> becomes read-only.
                </p>
                <p>
                    <strong>Settings &rarr; Providers</strong> has one section per
                    provider: its status and version, <strong>Sign in&hellip;</strong>
                    (which opens Terminal with the login command), whether it is
                    enabled, its executable if it is not on your <code>PATH</code>,
                    arguments (ACP agents), extra environment variables, accounts, and
                    the <strong>escalation model</strong> from chapter 9. Changes apply
                    to agents started afterwards.
                </p>
                <p>
                    <strong>Accounts</strong> let you use several logins for the same
                    agent, for example work and personal Claude subscriptions. Each is a
                    name and environment variables, separated by semicolons:
                </p>
                <pre><code>work: CLAUDE_CONFIG_DIR=~/.claude-work; personal: CLAUDE_CONFIG_DIR=~/.claude</code></pre>
                <p>
                    Pick the account before the first message; a session stays with the
                    account that started it. To log in to a new account, run the agent's
                    login with that account's environment, for Claude Code
                    <code>CLAUDE_CONFIG_DIR=~/.claude-work claude</code>, then
                    <code>/login</code>.
                </p>
                <p>
                    One more thing the table hides: Workspace uses the thread's provider
                    to write thread titles, commit messages, pull request descriptions
                    and recaps. Claude Code uses its fast model for this. If the
                    provider cannot do it, Workspace falls back to Claude Code, then
                    Elyra, if installed.
                </p>

                <h2 id="updates">Updates</h2>
                <p>Elyra Workspace updates itself:</p>
                <ol>
                    <li>At launch and every six hours, it checks GitHub for a newer release.</li>
                    <li>
                        It downloads the new version in the background and checks it:
                        the SHA-256 checksum must match, the app must be signed by the
                        same Developer ID team as the copy you run, Gatekeeper must
                        accept its notarization, and it must report the expected version.
                    </li>
                    <li>
                        A notification says the new version is ready. Click it to
                        restart into the new version. If agents are still working, you
                        choose: <strong>Restart when they finish</strong> waits and
                        restarts by itself once no turn is running; <strong>Restart
                        now</strong> stops them, and each thread can be resumed after the
                        update. If you do not click, the update installs the next time
                        you quit.
                    </li>
                </ol>
                <p>
                    <strong>Elyra Workspace &rarr; Check for Updates&hellip;</strong>
                    checks right away. To download only when you choose, turn off
                    <strong>Download and install updates automatically</strong> in
                    Settings &rarr; General. The app cannot replace itself when it runs
                    straight from the disk image or from a folder you cannot write to;
                    then the notification links to the download instead. Move the app to
                    Applications to get automatic updates.
                </p>

                <h3 id="going-back">Going back to the previous version</h3>
                <p>
                    Each update keeps the version it replaced, hidden beside the app. It
                    is not a second app in Spotlight or Launchpad.
                </p>
                <ul>
                    <li>
                        <strong>If a new version does not start:</strong> when it fails to
                        get through its first 20 seconds twice in a row (a crash, or it
                        hangs and you force it to quit), Elyra Workspace puts the previous
                        version back and starts it. A notification says so, and the broken
                        version is not offered again; the next release is.
                    </li>
                    <li>
                        <strong>By hand:</strong> command palette (<code>&#8984;K</code>)
                        &rarr; <strong>Go back to the previous version&hellip;</strong>, for
                        when a new version starts but something in it does not work for you.
                        The app restarts as the previous version; running agents stop and
                        can be resumed.
                    </li>
                </ul>
                <p>
                    Quitting normally within those 20 seconds counts as a good start.
                    This is the safety net that makes a self-updating app acceptable on a
                    machine that has agents working overnight.
                </p>

                <h2 id="privacy">Privacy: what leaves the Mac</h2>
                <ul>
                    <li>
                        Elyra Workspace has <strong>no account and sends no
                        telemetry</strong>.
                    </li>
                    <li>
                        <strong>The requests the app makes itself</strong> are the update
                        check and update downloads from GitHub, and two things you turn on:
                        pushes to the ntfy server you choose, if you enable Phone (chapter
                        13), and your Félagi workspace, if you connect it (chapter 12). The
                        browser tab loads the pages you open in it, like any browser.
                    </li>
                    <li>
                        <strong>Your messages and files go to the agents you use</strong>,
                        under their own terms. Workspace starts them as local programs.
                    </li>
                    <li>
                        The gateway listens only on <code>127.0.0.1</code> and requires a
                        token (chapter 14).
                    </li>
                    <li>
                        Pull requests, issues and other text from outside are marked for
                        the agent as reference, not instructions (chapter 5).
                    </li>
                </ul>

                <h2 id="shortcuts">Shortcuts</h2>
                <p>
                    Press <code>&#8984;/</code> to see all shortcuts, including any you
                    changed. To change them, press <strong>Edit&hellip;</strong> in that
                    sheet: it creates <code>~/.elyra/keybindings.json</code> with every
                    shortcut's id and default keys and opens it. Change the keys you want,
                    delete the lines you do not need, and restart Workspace. Keys are
                    written like <code>cmd-shift-p</code>, <code>ctrl-tab</code> or
                    <code>alt-down</code>, and <code>null</code> removes a shortcut.
                </p>

                <h2 id="problems">The problems that come up</h2>
                <table>
                    <thead><tr><th>Symptom</th><th>What to do</th></tr></thead>
                    <tbody>
                        <tr><td>An agent is listed as &ldquo;not installed&rdquo;</td><td>Workspace looks on your login shell's <code>PATH</code> and in the common install folders. Check <strong>Settings &rarr; Providers &rarr; Status</strong>; if it says <em>Not found</em>, set the full path under <em>Executable</em>.</td></tr>
                        <tr><td>The agent says it is not logged in, or fails at once</td><td>Press <strong>Sign in&hellip;</strong> in Settings &rarr; Providers, or run the agent once in a terminal. With accounts, sign in with that account's environment.</td></tr>
                        <tr><td>&ldquo;Elyra Workspace is already running&rdquo;</td><td>Only one copy runs per data folder. Switch to the open window, or use a different <code>ELYRA_HOME</code>.</td></tr>
                        <tr><td>A thread says the last turn was interrupted</td><td>The app quit while the agent was working. <strong>Resume</strong> or <strong>Dismiss</strong>. Next time, choose <strong>Quit when they finish</strong> (chapter 13).</td></tr>
                        <tr><td>The Changes tab says the folder is not a Git repository</td><td>Diffs, checkpoints, worktrees and pull requests need Git. Run <code>git init</code> in the project, or use it without those features.</td></tr>
                        <tr><td>The PR tab or review inbox shows nothing</td><td>They need the GitHub CLI: install it and run <code>gh auth login</code>. The project's remote must be on GitHub.</td></tr>
                        <tr><td>An automation did not run</td><td>Automations only run while the app is open. Check that it is switched on and look at its run history; a run is <em>Skipped</em> if the previous one is still going, and a run that waits for an approval is in its thread.</td></tr>
                        <tr><td>Option+key in the terminal does not type <code>[</code> or <code>@</code></td><td>Turn off <strong>Use Option as Meta key</strong> in Settings &rarr; Terminal.</td></tr>
                        <tr><td>After a crash</td><td>At the next launch Workspace shows a notice. Click it to open <code>~/.elyra/logs/crash.log</code>, and include it when you report a problem.</td></tr>
                    </tbody>
                </table>

                <h2 id="the-rhythm">What Freddy's week looks like now</h2>
                <p>
                    He does not work harder than he did with four terminal tabs; he
                    works with more of it visible. Monday morning, <strong>While you
                    were away</strong> shows what the weekend's automations did, and
                    the dependency run is a thread with a diff he reads at
                    <em>Agent turns</em> scope. He starts the day's three tasks as three
                    threads, two with their own worktree, and sets a check command so
                    that finished means green. One of them is a goal with a budget.
                    He asks for a flow to be tried in the browser and saved as a journey.
                    He leaves a thread to <em>Quit when they finish</em> at five, and
                    gets a push on the way home that says it passed.
                </p>
                <p>
                    None of that needs the agent to be cleverer than it was. It needs
                    the work to be somewhere you can see it, to be able to be undone,
                    and to be checked by something other than the agent's word. Those
                    are the three things the first chapter promised, and the fourteen
                    since have been the same three, looked at from different sides.
                </p>
                <div class="callout callout--success">
                    <strong>Try it:</strong> open Settings with <code>&#8984;,</code>
                    and read each page once. Then press <code>&#8984;/</code>, and
                    find two shortcuts you did not know.
                </div>
""",
        "learned": [
            "Where Workspace keeps its data, and how to run a separate copy",
            "What each Settings page is for",
            "How the providers differ, and what accounts are for",
            "How an update verifies itself, and how it goes back to the previous version",
            "What leaves the Mac, and which of those you turn on yourself",
            "The problems that come up most, and what to do about each",
        ],
        "next": "<strong>That is the course.</strong> The reference for everything here is the <a href=\"../overview.html\">Elyra Workspace documentation</a>: the <a href=\"../chat.html\">chat</a> and <a href=\"../projects-and-threads.html\">threads</a> guides, the <a href=\"../agent-gateway.html\">agent gateway</a> and <a href=\"../troubleshooting.html\">data, privacy and troubleshooting</a>. If you have not taken it, <em>Freddy the Notetaker</em> is the course where the app itself gets built, and <em>Freddy the Optimizer</em> is the one about making it found.",
    },
]

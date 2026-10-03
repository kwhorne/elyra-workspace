# Providers

A provider is the coding agent that runs a thread. Elyra Workspace drives the
command-line tools you install yourself and doesn't need API keys of its own.
Sign-in, billing and limits are handled by each tool.

| Provider | Command | How Elyra talks to it | Approvals | Effort | Fork | MCP gateway |
| --- | --- | --- | --- | --- | --- | --- |
| Claude Code | `claude` | stream-json | yes | low – max | yes | yes |
| Codex | `codex app-server` | JSON-RPC (app-server v2) | yes | minimal – extra high | yes | yes |
| Elyra | `elyra --mode rpc` | RPC | no (tools run directly) | off – extra high | yes | no |
| Pi | `pi --mode rpc` | RPC (same as Elyra) | no | off – extra high | yes | no |
| Gemini CLI | `gemini --experimental-acp` | Agent Client Protocol | yes | — | via context | yes |
| Cursor Agent | `cursor-agent acp` | Agent Client Protocol | yes | — | via context | yes |
| OpenCode | `opencode acp` | Agent Client Protocol | yes | — | via context | yes |
| Custom agent (ACP) | any command you set | Agent Client Protocol | yes | — | via context | yes |

*Fork via context* means a fork sends the earlier conversation along with its
first message, instead of branching the agent's own session. See
[Projects and threads](projects-and-threads.md#fork).

## Claude Code

The most complete integration:

- Its slash commands, skills and subagents appear under `/` and `@`.
- Models come from Claude Code itself.
- Plan mode shows the plan as a card.
- Questions the agent asks (AskUserQuestion) appear as forms.
- Messages can be sent into a running turn.
- `/compact` and cost per turn are supported.

Sessions are resumed by id, so a thread continues where it left off after a
restart. Sessions from Claude Code in the terminal can be imported with ⌘I.

## Codex

OpenAI's Codex runs through `codex app-server`:

- Replies and reasoning summaries stream in.
- Commands show their output live. File changes show as diffs.
- Task plans appear as a task card.
- When Codex asks a question, it appears as a form.
- Messages can be sent into a running turn (steer).
- `/compact` compacts the conversation.
- Models come from Codex itself. Effort sets the reasoning effort.
- Fork branches the Codex thread natively. Threads resume by id after a restart.

The permission mode maps to Codex's approval policy and sandbox:

| Elyra mode | Approval policy | Sandbox |
| --- | --- | --- |
| Ask for approval | untrusted | read-only |
| Accept edits | on-request | workspace-write |
| Plan only | never | read-only |
| Full access | never | danger-full-access |

Project instructions and debug mode are passed as developer instructions. Sign in
with `codex login` (or **Sign in…** in Settings). Codex sessions can be imported
with ⌘I ([Projects and threads](projects-and-threads.md#import-sessions)).

## Elyra and Pi

Both run tools without asking. When an extension needs an answer, it shows up as
a question card. Effort maps to the agent's thinking level. Messages can be sent
into a running turn, and `/compact` is supported.

## ACP agents: Gemini CLI, Cursor Agent, OpenCode and others

The [Agent Client Protocol](https://agentclientprotocol.com) is a common protocol
spoken by many agents. Through it Elyra supports:

- streaming replies and reasoning
- tool calls with diffs
- approval requests
- task plans
- the agent's commands
- model and mode lists
- resuming sessions, when the agent supports it

The permission mode is mapped to the agent's closest mode. For example *Plan
only* picks a mode called `plan`.

Any other ACP agent works as **Custom agent (ACP)**. Set its command, and any
arguments, in **Settings → Providers**. An adapter that wraps another agent also
works. For example, Claude Code through Zed's adapter:

- Command: `npx`
- Arguments: `-y @agentclientprotocol/claude-agent-acp`

## Provider settings

**Settings → Providers** has one section per provider:

- **Status**: whether the command was found, and its version. **Sign in…** opens
  Terminal with the provider's login command, using the environment set below.
- **Enabled**: switch a provider off to hide it from the agent menu. If the
  default provider is off, new threads use the first one that's on.
- **Executable**: a full path if the command isn't on your `PATH`. `~/` is
  allowed. For the custom agent, this is the command to run.
- **Arguments** (ACP agents): replaces the default arguments. Leave empty for the
  default.
- **Environment**: extra variables for every session, for example
  `HTTPS_PROXY=http://proxy:3128 API_KEY="value with spaces"`.
- **Accounts**: named sets of environment variables (see below).

Changes apply to agents started afterwards.

## Accounts

Accounts let you use several logins for the same agent, for example work and
personal Claude subscriptions. Each account is a name and environment variables,
separated by semicolons:

```text
work: CLAUDE_CONFIG_DIR=~/.claude-work; personal: CLAUDE_CONFIG_DIR=~/.claude
```

When a provider has accounts, an account picker appears below the message box.
Pick the account before the first message; a session stays with the account that
started it. To log in to a new account, run the agent's login with the account's
environment. For Claude Code, for example:

```console
CLAUDE_CONFIG_DIR=~/.claude-work claude
```

then use `/login`.

## Titles, commit messages and summaries

Elyra uses the thread's provider in one-shot mode to write:

- thread titles
- commit messages
- pull request descriptions
- recaps

Claude Code uses its fast model for this. If the thread's provider can't do this
(for example a custom ACP agent), Elyra falls back to Claude Code, then Elyra, if
installed.

# Agent gateway and MCP

Elyra Workspace includes a small [MCP](https://modelcontextprotocol.io) server.
Through it, agents can work with Elyra's threads. One agent can split a job into
parts, start a thread for each, wait for them and collect the results. Other apps
such as Claude Desktop or Codex can also control Elyra.

The server listens only on your own machine (`127.0.0.1`). Every caller needs a
token, and every call is written to an audit log. The address is shown in
**Settings → Agents & MCP**.

## Tools

| Tool | What it does | Read-only clients |
| --- | --- | --- |
| `list_projects` | Projects with id, name and path | ✓ |
| `list_threads` | Threads with status, optionally for one project and including archived ones | ✓ |
| `read_thread` | The conversation of a thread (most recent part) | ✓ |
| `wait_for_thread` | Wait until a thread finishes its turn or needs input, then return its status and last reply (default 10 minutes, at most 1 hour) | ✓ |
| `create_thread` | Start a thread in a project with a first message; optionally provider, model, title and a new worktree | |
| `send_message` | Send a message to a thread (queued if it's busy) | |
| `interrupt_thread` | Stop a thread's running turn | |
| `set_thread_title` | Rename a thread | |
| `archive_thread` | Archive a thread | |

Projects can be named by id, name or path. A thread can't wait for, interrupt or
archive itself.

## Letting agents manage threads

Turn on **Settings → Agents & MCP → Let agents manage threads**. Agents started
afterwards get the tools above as an MCP server named `elyra`. In Claude Code they
appear as `mcp__elyra__…`.

- Supported by Claude Code, Codex and ACP agents. Elyra and Pi don't support MCP
  servers.
- Tool calls follow the thread's permission mode like any other tool. In *Ask for
  approval* mode you approve each one.
- Each agent gets its own token, which only lasts while Elyra is running.

Example prompt:

> Split the migration into three parts. For each, create a thread in this project
> with a worktree, then wait for all three and summarize what each did.

The gateway is off by default, because agents that start other agents can use a
lot of tokens quickly.

## Connecting Claude Desktop, Codex or Claude Code

Under **External clients** in Settings:

1. Press **Pair read-only client** or **Pair full-access client**. Read-only
   clients can list, read and wait, but not change anything.
2. Use the copy buttons on the new client:
   - **Copy Claude Desktop config**: JSON for `claude_desktop_config.json`
   - **Copy Codex config**: a block for `~/.codex/config.toml`
   - **Copy Claude Code command**: a `claude mcp add …` command to run in a
     terminal
3. Paste it, and restart the client if it needs that.

Claude Desktop and Codex start Elyra's built-in bridge, which passes their
requests to the running app:

```console
"/Applications/Elyra Workspace.app/Contents/MacOS/elyra" mcp-bridge <url> <token>
```

Elyra Workspace must be running for this to work. The server's port is kept
between launches, so pasted configs keep working.

**Revoke** (trash icon) removes a client; its token stops working at once. The
list shows when each client was last used.

## Audit log

**Activity** in the same Settings page lists the latest calls, newest first: time,
caller (a client's name, or *agent: thread title*), tool, arguments, and whether
it succeeded. Elyra keeps the latest 5,000 entries.

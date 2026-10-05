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
| `browser_open` | Open a local development page in a thread's [browser](browser.md) | |
| `browser_snapshot` | The page's structure as an outline of its elements and text | ✓ |
| `browser_query` | Elements matching a CSS selector, with position, size and computed styles | ✓ |
| `browser_console` | What the page wrote to the console since it loaded | ✓ |
| `browser_network` | The page's fetch and XHR calls (status, timing, start of the body) and the files it loaded | ✓ |
| `browser_screenshot` | A picture of the page (the Browser tab must be on screen) | ✓ |
| `browser_reload` | Reload the page | |

Projects can be named by id, name or path. A thread can't wait for, interrupt or
archive itself.

The browser tools work on the caller's own thread; other clients pass
`thread_id`. They only open and read pages served from this Mac (`localhost`,
`127.0.0.1`, `*.test`, `*.local` and similar); see [Browser](browser.md).

## Letting agents manage threads

Turn on **Settings → Agents & MCP → Let agents manage threads**. Agents started
afterwards get the tools above as an MCP server named `elyra`. In Claude Code they
appear as `mcp__elyra__…`.

- Supported by Claude Code, Codex, Elyra (0.9.46 or later) and ACP
  agents. Pi can use MCP servers through an extension with its own
  configuration, but doesn't get the gateway from Elyra Workspace yet.
- With Elyra 0.9.47 or later the gateway's tools are there from the first message
  of a thread. With 0.9.46 Elyra registers them in the background, so an agent may
  have to reach them through `mcp_search` and `mcp_call` first.
- Tool calls follow the thread's permission mode like any other tool. In *Ask for
  approval* mode you approve each one.
- An agent may read any thread, but the first time it messages, stops, renames
  or archives *another* thread, or opens or reloads a page in its browser, Elyra
  asks you: **Don't allow**, **Allow once** or
  **Always allow**. *Always* remembers that pair of threads; **Settings → Agents &
  MCP → Agents changing other threads** shows how many are remembered and forgets
  them. Threads an agent started with `create_thread` are its own, so it isn't
  asked about those. Paired clients such as Claude Desktop aren't asked: pairing
  them was the permission.
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

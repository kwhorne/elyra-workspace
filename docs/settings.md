# Settings

Open Settings with **⌘,**, the gear in the title bar, or **Elyra Workspace →
Settings…**. Changes apply right away and are saved automatically. Use the search
field to find a setting. Every page except *Agents & MCP* has a reset button
that restores its defaults.

## General

| Setting | Meaning |
| --- | --- |
| External editor → Open projects in | The editor ⌘O uses. *Automatic* picks the first one installed. |
| Updates → Check for updates automatically | Looks for a newer release on GitHub at launch and every six hours. **Elyra Workspace → Check for Updates…** checks now. |
| Updates → Download and install updates automatically | Downloads and verifies a new version in the background; it installs when you restart or quit. Off: you're told about the new version and click to install it. See [Updates](troubleshooting.md#updates). |
| Phone → Push to the phone while you're away | Pushes to your phone through ntfy when a thread needs you or finishes while Elyra Workspace isn't the active app, and takes your answers. Off by default. See [On your phone](chat.md#on-your-phone). |
| Phone → ntfy server, Topic | The ntfy server (`https://ntfy.sh` or your own) and the private topic, made for you when you turn it on. **Send a test push** checks it. |

## Appearance

| Setting | Meaning |
| --- | --- |
| Color theme | Default Dark, Default Light, Tokyo Night, Palenight, Material, Dracula, Nord, plus your custom themes. The terminal colors follow the theme. |
| Follow system appearance | Switch between the light and dark theme below when macOS switches. |
| Light theme / Dark theme | The themes used for light and dark. ⇧⌘T switches between them by hand, which turns *Follow system appearance* off. |
| Custom themes | **Open folder** opens `~/.elyra/themes`. Theme files there (gpui-component theme JSON) are loaded at startup. |
| Interface font | Font family and size for the whole interface. The size scales everything. |
| Conversation width | Maximum width of the conversation in points (default 860). 0 uses the full width. |
| Density | *Comfortable* or *Compact* spacing in the conversation. |

## Code

Font family and size for diffs, tool output, the editor and code in the
conversation. The list shows the monospaced fonts installed on your Mac (Menlo,
Monaco, SF Mono, JetBrains Mono, Fira Code, …).

**Commits and pull requests → Conventional format** (on by default): commit
messages and pull request titles as `type(scope): summary`, such as
`fix(nightwatch-exceptions): employee service create`. Generated ones follow it,
and committing or creating a pull request checks it. See
[Conventional Commits](git-and-review.md#conventional-commits).

## Terminal

| Setting | Meaning |
| --- | --- |
| Font family / Font size / Line height | *Same as code font* by default |
| Cursor shape | Block, Bar or Underline. Programs like vim can still change it. |
| Blinking cursor | |
| Use Option as Meta key | Off: Option types characters (needed for `[ ] { } \| @ ~` on Nordic keyboards). On: Option+key sends Esc+key for Emacs and readline. |
| Copy on select | Copy selected text to the clipboard right away |
| Shell command | Leave empty for your login shell. Applies to new terminals. |
| Scrollback lines | How much output each terminal keeps (default 10,000) |

## Providers

One section per agent: status and version, **Sign in…**, enabled, executable,
arguments (ACP agents), environment, accounts, and the **escalation model** a
fix gets when the [checks](context-and-goals.md#checks-after-each-turn) still
fail after a first automatic fix. See [Providers](providers.md).

## Agents & MCP

| Setting | Meaning |
| --- | --- |
| Let agents manage threads | Gives Claude Code, Codex, Elyra and ACP agents tools to list, read, create and steer threads and to look at local pages in the browser ([Agent gateway](agent-gateway.md)). Off by default. |
| Server | The address of Elyra's local MCP server |
| Grove runs worktrees | In Grove apps, new worktrees come from Grove with their own database copy and address ([Worktrees](git-and-review.md#worktrees-with-their-own-running-app)). On by default. |
| Database in checkpoints | In Grove apps, each checkpoint also snapshots the database, and restoring puts it back ([Checkpoints](chat.md#checkpoints)). On by default. |
| Grove tools for agents | In projects [Elyra Grove](context-and-goals.md#grove) runs as an app, agents also get Grove's MCP server, read-only: sites, recent requests, request chains and explanations, logs and database schema. On by default. |
| Automatic turns per goal | How many turns a [goal](context-and-goals.md#goal) may take on its own before it pauses (default 10) |
| Automatic fixes when checks fail | How many times a project's failed [checks](context-and-goals.md#checks-after-each-turn) go back to the agent before the thread is handed to you (default 2; 0 only reports them) |
| External clients | Pair, configure and revoke Claude Desktop, Codex and other MCP clients |
| Activity | The audit log of gateway calls |

## Félagi

**Connection**: the [Elyra Félagi](automations-and-tasks.md#félagi-tasks-and-time)
workspace the task board's Félagi tab and thread reports talk to, and as whom.
**Connect…** asks for the address and a personal token (kept in the macOS
Keychain); **Disconnect** removes it.

**Run Félagi's agents on this Mac** ([how it works](automations-and-tasks.md#running-félagis-agents-on-this-mac)):

| Setting | Meaning |
| --- | --- |
| Run agents here | Claim and run the work Félagi gives agents whose runtime is this Mac. Off by default. |
| Machine name | How this Mac is listed among Félagi's runtimes; make it unique. Blank uses the computer's name. |
| Daemon token | The `fdt_…` token from Félagi → Admin → Runtimes, kept in the Keychain. |
| Status | What the runtime is doing, and the last problem. |

## Settings that follow your use

Some defaults aren't on a settings page; Elyra takes them from what you do:

- New threads use the agent, model, effort and permission mode you chose last.
- The open tabs, window size and position, sidebar and tools panel, the open tool
  tab, collapsed projects and the selected space are restored at launch.

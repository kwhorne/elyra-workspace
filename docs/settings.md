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

## Appearance

| Setting | Meaning |
| --- | --- |
| Color theme | Default Dark, Default Light, Tokyo Night, Palenight, Dracula, Nord, plus your custom themes. The terminal colors follow the theme. |
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
arguments (ACP agents), environment and accounts. See [Providers](providers.md).

## Agents & MCP

| Setting | Meaning |
| --- | --- |
| Let agents manage threads | Gives Claude Code, Codex, Elyra and ACP agents tools to list, read, create and steer threads and to look at local pages in the browser ([Agent gateway](agent-gateway.md)). Off by default. |
| Server | The address of Elyra's local MCP server |
| Automatic turns per goal | How many turns a [goal](context-and-goals.md#goal) may take on its own before it pauses (default 10) |
| External clients | Pair, configure and revoke Claude Desktop, Codex and other MCP clients |
| Activity | The audit log of gateway calls |

## Settings that follow your use

Some defaults aren't on a settings page; Elyra takes them from what you do:

- New threads use the agent, model, effort and permission mode you chose last.
- The open tabs, window size and position, sidebar and tools panel, the open tool
  tab, collapsed projects and the selected space are restored at launch.

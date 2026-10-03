# Keyboard shortcuts

Press **⌘/** in the app to see all shortcuts, including any you changed.

## General

| Shortcut | Action |
| --- | --- |
| ⌘K | Command palette (in a terminal, ⌘K clears it) |
| ⌘, | Settings |
| ⌘/ | Keyboard shortcuts |
| ⇧⌘T | Toggle light/dark theme |
| ⌘Q | Quit (asks first if agents are running) |

## Threads

| Shortcut | Action |
| --- | --- |
| ⌘N | New thread in the current project |
| ⌥⌘N | New chat without a project |
| ⌘W | Close tab |
| ⌘1 … ⌘8, ⌘9 | Go to tab 1–8, last tab |
| ⇧⌘] / ⇧⌘[ | Next / previous tab |
| ⌃Tab | Most recent thread |
| ⌘[ / ⌘] | Back / forward |
| ⌘\\ | Split chat |
| ⌥⌘S | New side chat |
| ⇧⌘K | Fork thread |
| ⌘I | Import from Claude Code |
| ⌘L | Focus the message box |
| ⌘F | Find in thread (message box focused) |
| ⌃C | Stop the agent (message box focused) |
| ⇧⌘R | Code review inbox |

## Projects and files

| Shortcut | Action |
| --- | --- |
| ⇧⌘O | Add project |
| ⌘O | Open in external editor |
| ⌘P | Find file |
| ⇧⌘F | Search in files |
| ⌘S | Save (Files tab) |

## Panels

| Shortcut | Action |
| --- | --- |
| ⌘B | Toggle sidebar |
| ⌥⌘B | Toggle tools panel |
| ⇧⌘G | Changes |
| ⇧⌘E | Files |
| ⇧⌘I | Context and notes |
| ⌘J | Toggle terminal |
| ⇧⌘J | Full-width terminal |
| ⌥⌘T | Task board |
| ⌥⌘A | Automations |

## Git

| Shortcut | Action |
| --- | --- |
| ⌃⌘P | Commit and push |
| ⌥↓ / ⌥↑ | Next / previous change in a diff |

## Terminal

| Shortcut | Action |
| --- | --- |
| ⌘T | New terminal |
| ⇧⌘W | Close terminal |
| ⇧⌘] / ⇧⌘[ | Next / previous terminal |
| ⌘D | Split terminal |
| ⌘F | Search scrollback (Enter / ⌘G older, ⇧Enter / ⇧⌘G newer) |
| ⌘K | Clear |
| ⌘C / ⌘V / ⌘A | Copy / paste / select all |

More terminal keys are in [Terminal](terminal.md#keys).

## Message box

| Keys | Action |
| --- | --- |
| Enter | Send (or queue while the agent works) |
| Shift+Enter | New line |
| ↑ / ↓ (empty box) | Earlier messages |
| / | Commands |
| @ | Mention a file or subagent |
| ↑ / ↓, Enter or Tab, Escape | Move, pick and close in the `/` and `@` lists |

## Changing shortcuts

Shortcuts are read from `~/.elyra/keybindings.json` at launch. In the shortcuts
sheet (⌘/), **Edit…** creates the file with every shortcut's id and default keys,
and opens it. Change the keys you want, delete the lines you don't need, and
restart Elyra Workspace.

```json
{
  "find_file": "cmd-shift-p",
  "command_palette": "cmd-k",
  "terminal": null
}
```

- Keys are written like `cmd-shift-p`, `ctrl-tab`, `alt-down`. Modifiers are
  `cmd`, `ctrl`, `alt` and `shift`. Separate a sequence of keystrokes with a
  space.
- `null` or `""` removes a shortcut.
- Ids you don't list keep their default. Invalid keys are skipped and noted in the
  log.

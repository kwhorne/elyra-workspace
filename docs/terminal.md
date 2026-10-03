# Terminal

Every thread has its own terminals, which start in the thread's folder (its
worktree, if it has one). Open them with **⌘J**, or the **Terminal** tab in the
tools panel. **⇧⌘J** gives the terminal the whole window; press it again to go
back.

The terminal runs your login shell (or the shell set in Settings) and is complete
enough for full-screen programs such as vim, htop, lazygit and Claude Code itself.

## Tabs and splits

| Shortcut | Action |
| --- | --- |
| ⌘T | New terminal tab |
| ⇧⌘W | Close terminal |
| ⇧⌘] / ⇧⌘[ | Next / previous terminal |
| ⌘D | Split: the current terminal stays on the left, a new one opens on the right. Press again to unsplit. |

The tab title follows the program running in it. A terminal whose shell has
exited is shown struck through. If a command is still running when you close a
terminal, Elyra asks first.

## Scrollback search

**⌘F** opens a search bar above the terminal. The newest match is selected and
scrolled into view:

- **Enter** or **⌘G**: the next older match
- **Shift+Enter** or **⇧⌘G**: the next newer match
- **Escape**: close

Searching ignores case unless your text contains a capital letter.

## Selecting, copying and pasting

- Drag to select. Double-click selects a word, triple-click a line, and ⌥-drag a
  rectangular block.
- **⌘C** copies and **⌘V** pastes. Pasting uses bracketed paste when the program
  asks for it. **⌘A** selects everything.
- With **Copy on select** (Settings → Terminal), selecting copies right away.
- The **Add selection to chat** button in the terminal's toolbar puts the selected
  text in the thread's message box as a code block. Use it to show the agent an
  error.

## Keys

| Keys | Action |
| --- | --- |
| ⌘K | Clear the screen and scrollback (in a terminal, ⌘K does this instead of opening the command palette) |
| ⌘← / ⌘→ | Start / end of line |
| ⌘⌫ | Delete to the start of the line |
| ⌥← / ⌥→ | Previous / next word |
| ⇧PgUp / ⇧PgDn, ⌘↑ / ⌘↓ | Scroll |
| ⌘-click | Open a link |

**Option** types characters by default, as macOS does, which you need for
`[ ] { } | @ ~` on Norwegian and other Nordic keyboards. Turn on **Use Option as
Meta key** in Settings → Terminal if you use Emacs-style Meta shortcuts instead.

## Mouse and full-screen programs

Programs that use the mouse (vim, htop, tmux…) receive clicks, drags and scrolling.
Hold **Shift** to select text anyway. The terminal also supports:

- the alternate screen used by full-screen programs
- cursor shapes and blinking set by the program
- wide characters (CJK and emoji)
- clickable links
- the window title set by the program
- copying to the clipboard from programs (OSC 52)
- typing accented characters with dead keys and input methods

Font, size, line height, cursor, shell and scrollback length are set in
**Settings → Terminal**.

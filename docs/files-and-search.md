# Files and search

## Command palette (⌘K)

The palette searches everything at once:

- **Threads** by title. With no search text, your eight most recent threads are
  listed. From three characters, the threads' **conversations** are searched too
  (best match first, like [Search threads](#search-threads-f)), with an excerpt.
- **Commands**: every action in the app, with its shortcut.
- **New thread in…**: start a thread in a project.
- **Theme: …**: switch color theme.

Use **↑/↓** (or **⌃N / ⌃P**) to move, **Enter** to pick, **Escape** or a click
outside to close. Matching is fuzzy: `nwthr` finds *New thread*.

## Search threads (⌥⌘F)

Searches every thread at once: your messages, the agents' replies, their tool
calls and the start of what the tools printed (the first 8,000 characters of
each result). Use it to find how something was solved before, an error you've
seen, or the thread where a decision was made.

- Type words: each must occur, and each matches the start of a word, so
  `deploy` also finds *deployment* and *deployed*. Case and accents don't matter.
- Put words in double quotes for an exact phrase: `"rounding error"`.
- Hits are grouped by thread, best match first. Each row says what the hit is
  in (**You**, **Agent**, **Tool**, **Output**), shows an excerpt with the
  matching words marked, and the date. Click one to open the thread scrolled to
  that entry.
- **Tool output** (on by default) includes tool calls and their output; switch it
  off to search only what was said. **Archived** includes archived threads. The
  project menu limits the search to one project.

The search is fast also with many threads: Elyra keeps a full-text index of the
conversations in its database and updates it as threads change. Agents can
search the same way with the gateway's `search_threads` tool
([Agent gateway](agent-gateway.md)).

## Find a file (⌘P)

Lists the files of the active thread's folder, or the current project's,
respecting `.gitignore`. Type part of a name or path. Picking a file opens it in
the Files tab.

## Go to symbol (⌘T)

Finds a function, method, class, type or module by name in the active thread's
folder, or the current project's, and opens its file at that line in the Files
tab. Matching is fuzzy like **Find a file**: `updtot` finds `update_totals`, and
an exact name comes first. Each row shows the kind, the class or module it
belongs to, and `file:line`.

It reads the same symbol index the agents use with the gateway's `symbols` tool:
Rust, TypeScript, TSX, JavaScript, Python, Go, PHP and C, following `.gitignore`.
The first time in a project the index is built (a few seconds for a few thousand
files); after that it's refreshed in the background as files change.

## Search in files (⇧⌘F)

Searches file contents in the same folder. From two characters, results show the
matching line with its file and line number. Picking one opens the file at that
line. The search is case-insensitive unless your text contains a capital letter.
It skips binary files, files over 2 MB and anything ignored by `.gitignore`.

## The Files tab (⇧⌘E)

A file tree and an editor for the active thread's folder, in the tools panel.

- **Tree**: folders first, then files. `.gitignore`d files are hidden, other
  dotfiles are shown. Click a folder to expand it. Hide or show the tree with the
  panel button in the header.
- **Editor**: syntax highlighting for common languages, with line numbers.
  - **Autosave**: edits are saved shortly after you stop typing. **⌘S** saves at
    once. The header shows *Unsaved* or *Saved*.
  - **Changes on disk**: if the agent (or anything else) changes the file, the
    editor reloads it, unless you have unsaved edits. Then a bar offers
    **Reload** (take the disk version) or **Overwrite** (keep yours).
  - **Markdown**: the eye button switches between editing and a rendered
    preview.
  - **Images** (PNG, JPEG, GIF, WebP, SVG, BMP, ICO) are shown as a preview.
    Binary files and files over 2 MB are not opened.
- **@** in the header adds `@path` for the open file to the thread's message box.
- The refresh button re-reads the tree and checks the open file. The tree also
  refreshes after every agent turn.

### Large editor (⌥⌘E)

The tools panel is narrow for editing. The expand button in the Files header
(or **⌥⌘E**) slides the tree and editor in from the right over half the window;
drag its edge to make it wider. The conversation stays visible beside it. Close
it with **×**, **⌥⌘E** or a click outside it; the file stays open in the Files
tab.

## Open in an external editor (⌘O)

**⌘O** opens the thread's folder in your editor. If a file is open in the Files
tab, it opens that file instead. The **Open** button in the title bar shows your
editor; its menu lists every supported editor found on this Mac:

VS Code, Cursor, Windsurf, Zed, Sublime Text, Nova, Xcode, IntelliJ IDEA,
PhpStorm, WebStorm, PyCharm, RustRover, GoLand, Fleet, BBEdit, Terminal, iTerm,
Ghostty, Warp and Finder.

Choose your default in **Settings → General → External editor**. *Automatic*
picks the first one installed.

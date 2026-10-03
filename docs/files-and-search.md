# Files and search

## Command palette (⌘K)

The palette searches everything at once:

- **Threads** by title. With no search text, your eight most recent threads are
  listed. From three characters, matching **message text** is searched too.
- **Commands**: every action in the app, with its shortcut.
- **New thread in…**: start a thread in a project.
- **Theme: …**: switch color theme.

Use **↑/↓** (or **⌃N / ⌃P**) to move, **Enter** to pick, **Escape** or a click
outside to close. Matching is fuzzy: `nwthr` finds *New thread*.

## Find a file (⌘P)

Lists the files of the active thread's folder, or the current project's,
respecting `.gitignore`. Type part of a name or path. Picking a file opens it in
the Files tab.

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

## Open in an external editor (⌘O)

**⌘O** opens the thread's folder in your editor. If a file is open in the Files
tab, it opens that file instead. The **Open** button in the title bar shows your
editor; its menu lists every supported editor found on this Mac:

VS Code, Cursor, Windsurf, Zed, Sublime Text, Nova, Xcode, IntelliJ IDEA,
PhpStorm, WebStorm, PyCharm, RustRover, GoLand, Fleet, BBEdit, Terminal, iTerm,
Ghostty, Warp and Finder.

Choose your default in **Settings → General → External editor**. *Automatic*
picks the first one installed.

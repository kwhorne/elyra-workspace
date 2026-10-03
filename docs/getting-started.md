# Getting started

## Requirements

- A Mac with Apple Silicon (M1 or later), macOS 12 or later. Intel Macs are not
  supported.
- `git`
- At least one coding agent installed and signed in:

| Agent | Install | Sign in |
| --- | --- | --- |
| Claude Code | `npm install -g @anthropic-ai/claude-code` | run `claude` and use `/login` |
| Elyra | `npm install -g @elyracode/coding-agent` | run `elyra` |
| Pi | `npm install -g @mariozechner/pi-coding-agent` | run `pi` |
| Gemini CLI | `npm install -g @google/gemini-cli` | run `gemini` |
| Cursor Agent | `curl https://cursor.com/install -fsS \| bash` | `cursor-agent login` |
| OpenCode | `npm install -g opencode-ai` | `opencode auth login` |

Elyra finds these on your `PATH`, including the usual per-user install folders
(`~/.npm-global/bin`, `~/.local/bin`, `~/.bun/bin`, Homebrew). If one lives somewhere
else, set its path in **Settings → Providers** ([Providers](providers.md)). You can
also sign in from there.

## Install

Build the app bundle from the source folder:

```console
scripts/bundle-macos.sh
```

This creates `target/release/bundle/Elyra Workspace.app`. Drag it to
`/Applications`, or run it from where it is. To run without a bundle, use
`cargo run --release`.

Only one copy of Elyra Workspace runs at a time per data folder. If you start a
second copy, it exits.

## First launch

A welcome dialog lists the supported agents and shows which ones are installed,
with their versions. Here you can also choose a color theme. Then either:

- **Add a project**: pick a folder. Elyra opens a new thread in it.
- **Start a chat**: opens a thread in a scratch folder (`~/.elyra/chats`). Use it
  for questions that don't belong to a project.

You can add more projects at any time with ⇧⌘O or the folder button in the
sidebar.

## Your first thread

1. Select a project and press **⌘N** (New thread).
2. Below the message box, check the agent, model, effort and permission mode. The
   defaults are the ones you used last. See [Working with an agent](chat.md).
3. If the project is a Git repository, choose **Local** or **New worktree**:
   - **Local**: the agent works in the project folder itself.
   - **New worktree**: the agent gets its own copy on its own branch, so several
     threads can work in parallel without getting in each other's way.
4. Write what you want and press **Enter**. **Shift+Enter** starts a new line.
5. Watch the agent work. Tool calls appear as they run, and edits show as diffs.
   Depending on the permission mode, the agent may stop and ask for approval.
6. Open the tools panel (⌥⌘B) to see the **Changes** the agent made, and the
   terminal (⌘J).

When the agent finishes, the thread's status in the sidebar changes. If you were
looking at another thread, you get a notification. If the window was in the
background, it's a system notification and the Dock icon shows a badge.

## Next steps

- Press **⌘K** for the command palette: it finds threads, files and commands.
- Press **⌘/** to see every keyboard shortcut.
- Open **Settings** with **⌘,**.

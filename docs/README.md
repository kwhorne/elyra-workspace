# Elyra Workspace user guide

Elyra Workspace is a desktop app for working with coding agents. You add your
project folders, start a thread for each task, and talk to an agent such as Claude
Code. The changes it makes, Git, a terminal and your files are all in the same
window. Everything runs on your Mac. Conversations are stored in a local database,
and the agents run as the command-line tools you already have installed.

## Contents

| Guide | What it covers |
| --- | --- |
| [Getting started](getting-started.md) | Installing, first launch, your first project and thread |
| [Projects and threads](projects-and-threads.md) | The sidebar, tabs, spaces, archiving, forking, side chats, import and export |
| [Working with an agent](chat.md) | The composer, attachments, `@` and `/`, models, approvals, plans, checkpoints |
| [Providers](providers.md) | Claude Code, Codex, Elyra, Pi, Gemini CLI, Cursor, OpenCode and other ACP agents; accounts |
| [Changes, Git and review](git-and-review.md) | Diffs, staging, commits, branches, worktrees, pull requests, the review inbox |
| [Files and search](files-and-search.md) | Command palette, file finder, content search, the editor, external editors |
| [Browser](browser.md) | The built-in browser for your dev servers, and letting agents look at pages |
| [Terminal](terminal.md) | Terminal tabs and splits, search, full-screen programs, copy and paste |
| [Context, notes and goals](context-and-goals.md) | Notes, pinned messages, recaps, project instructions, dev servers, goals |
| [Automations and tasks](automations-and-tasks.md) | Scheduled prompts and the task board |
| [Agent gateway and MCP](agent-gateway.md) | Letting agents manage threads, connecting Claude Desktop and Codex |
| [Settings](settings.md) | Every page of the Settings window |
| [Keyboard shortcuts](keyboard-shortcuts.md) | All shortcuts and how to change them |
| [Data, privacy and troubleshooting](troubleshooting.md) | Where data lives, logs, common problems |

Building Elyra Workspace yourself, or contributing? See [Development](development.md).

## The window at a glance

```
┌──────────────┬──────────────────────────────┬─────────────────────────┐
│ Sidebar      │ Thread tabs                  │ Tools panel             │
│              │                              │ Changes · PR · Files ·  │
│ Projects     │ Conversation                 │ Browser · Context ·     │
│  └ threads   │                              │ Terminal · Side chat    │
│              │                              │                         │
│              │ Composer                     │                         │
└──────────────┴──────────────────────────────┴─────────────────────────┘
```

- **Sidebar** (⌘B): projects and their threads, pinned threads, project spaces, and
  buttons for the task board, automations, code review, new chat and adding a project.
- **Center**: the open threads as tabs. The task board, automations, the review
  inbox and usage statistics open here as well.
- **Tools panel** (⌥⌘B): tools for the active thread.

## Core ideas

- **Project**: a folder on your disk, usually a Git repository.
- **Thread**: one task. It has its own conversation, agent session, model and
  settings. It works either in the project folder itself or in its own Git worktree.
- **Provider**: the coding agent that runs the thread, such as Claude Code. Each
  thread uses one provider. You can fork a thread or hand it off to another
  provider.
- **Turn**: one message from you and the agent's work until it stops. Before every
  turn, Elyra saves a checkpoint of your files so you can go back.

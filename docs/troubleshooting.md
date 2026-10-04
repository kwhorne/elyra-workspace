# Data, privacy and troubleshooting

## Where your data lives

Everything is stored in `~/.elyra`:

| Path | Contents |
| --- | --- |
| `state.db` | Projects, threads, conversations, settings, automations, tasks, paired clients and the audit log (SQLite) |
| `worktrees/` | Worktrees Elyra created for threads |
| `chats/` | The scratch folder used by chats without a project |
| `themes/` | Your custom themes |
| `keybindings.json` | Your shortcut overrides, if any |
| `logs/crash.log` | Details of crashes |

Checkpoints are hidden Git references (`refs/elyra/…`) inside each project's
repository. Deleting a thread removes them.

To run a separate copy with its own data, for example to try something out, set
`ELYRA_HOME`:

```console
ELYRA_HOME=~/elyra-test "/Applications/Elyra Workspace.app/Contents/MacOS/elyra"
```

## Updates

Elyra Workspace updates itself:

1. At launch and every six hours, it checks GitHub for a newer release.
2. It downloads the new version in the background and checks it:
   - the SHA-256 checksum must match
   - the app must be signed by the same Developer ID team as the copy you run
   - Gatekeeper must accept its notarization
   - it must report the expected version
3. A notification says the new version is ready. Click it to restart into the
   new version. If agents are still working, you choose: **Restart when they
   finish** waits and restarts by itself once no turn is running; **Restart now**
   stops them, and each thread can be resumed after the update. If you don't
   click, the update installs the next time you quit.

**Elyra Workspace → Check for Updates…** checks right away and shows the
progress. To download only when you choose, turn off **Download and install
updates automatically** in Settings → General.

The app can't replace itself when it runs straight from the disk image, or from a
folder you can't write to. Then the notification links to the download instead.
Move the app to Applications to get automatic updates. Versions before 0.1.2 only
link to the download, so update those once by hand.

## Privacy

- Elyra Workspace has no account and sends no telemetry. The only network requests
  it makes itself are the update check and update downloads from GitHub.
- Your messages and files go to the agents you use, under their own terms. Elyra
  starts them as local programs.
- The MCP server listens only on `127.0.0.1` and requires a token.
- Pull requests, issues and other text from outside are marked for the agent as
  reference, not instructions.

## Common problems

**An agent is listed as "not installed".**
Elyra looks on your login shell's `PATH` and in the common install folders. Check
**Settings → Providers → Status**. If it says *Not found*, set the full path under
*Executable*.

**The agent says it isn't logged in, or fails at once.**
Press **Sign in…** in Settings → Providers, or run the agent once in a terminal to
log in. If you use accounts, sign in with that account's environment
([Accounts](providers.md#accounts)).

**"Elyra Workspace is already running".**
Only one copy runs per data folder. Switch to the open window, or use a different
`ELYRA_HOME`.

**A thread says the last turn was interrupted.**
The app quit while the agent was working. Press **Resume** to continue, or
**Dismiss**.

**The Changes tab says the folder is not a Git repository.**
Diffs, checkpoints, worktrees and pull requests need Git. Run `git init` in the
project, or use it without these features.

**The PR tab or review inbox shows nothing.**
They need the [GitHub CLI](https://cli.github.com): install it and run
`gh auth login`. The project's remote must be on GitHub.

**An automation didn't run.**
Automations only run while the app is open. Check that the automation is
switched on and look at its run history. A run is *Skipped* if the previous one
is still going. If a run waits for an approval, open its thread.

**Option+key in the terminal doesn't type `[` or `@`.**
Turn off **Use Option as Meta key** in Settings → Terminal.

**After a crash.**
At the next launch, Elyra shows a notice. Click it to open
`~/.elyra/logs/crash.log`; include it when reporting a problem.

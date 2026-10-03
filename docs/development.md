# Development

Conventions for contributors and coding agents are in [AGENTS.md](../AGENTS.md).

## Requirements

- A Mac with Apple Silicon running macOS 12 or later (the only supported platform)
- Rust 1.85 or later (edition 2024)
- `git`, and `python3` for the provider adapter tests
- For a live run: at least one agent CLI ([Providers](providers.md))

## Build and run

```console
cargo run --release            # run the app
scripts/bundle-macos.sh        # -> target/release/bundle/Elyra Workspace.app
```

The bundle is signed ad hoc unless `CODESIGN_IDENTITY="Developer ID Application: …"`
is set. With an identity it is signed with the hardened runtime and a secure
timestamp, as notarization requires. Bundle id: `com.gets.elyra-workspace`. The
script refuses to build on anything other than Apple Silicon.

## Checks

```console
cargo fmt --all
cargo clippy --workspace --all-targets   # zero warnings
cargo test --workspace
```

For UI changes, also run the app with an isolated data directory and look at it.

## Environment variables

| Variable | Effect |
| --- | --- |
| `ELYRA_HOME=<dir>` | Use `<dir>` instead of `~/.elyra` (isolated instances; never test against your real data) |
| `ELYRA_NO_ACTIVATE=1` | Open the window in the background without taking focus |
| `ELYRA_OPEN_PANEL=tasks\|automations\|stats\|review` | Open that panel at launch (screenshots without input) |
| `ELYRA_BROWSER_URL=<url>` | Open `<url>` in the active thread's browser at launch |
| `ELYRA_UPDATE_RESTART_WHEN_READY=1` | Restart into a staged update as soon as it is ready (end-to-end update tests) |
| `RUST_LOG=debug` | More logging |

`elyra mcp-bridge <url> <token>` runs the stdio bridge for external MCP clients
instead of the app.

## Tools for checking providers

```console
# A real turn through any adapter, printing the neutral events
cargo run -p elyra-provider --example smoke -- claude <dir> "<prompt>"
cargo run -p elyra-provider --example smoke -- codex <dir> "<prompt>"

# Any ACP agent (ACP_COMMAND overrides the executable and arguments)
ACP_COMMAND="npx -y @agentclientprotocol/claude-agent-acp" \
  cargo run -p elyra-provider --example smoke -- acp <dir> "<prompt>"

# Sessions that can be imported
cargo run -p elyra-provider --example claude_history
cargo run -p elyra-provider --example codex_history [path/to/codex]

# GitHub integration (read-only)
cargo run -p elyra-git --example gh_smoke
```

The ACP and Codex adapters are tested against small stand-in agents in
`crates/elyra-provider/tests/` (`mock_acp_agent.py`, `mock_codex_app_server.py`).
Extend these when an adapter handles new messages.

## Architecture

```text
crates/
├── elyra-core      domain model, SQLite store and migrations, orchestration records
│                   (automations, tasks, MCP clients, audit log), schedules, data paths
├── elyra-provider  AgentSession trait and provider-neutral events; adapters for
│                   Claude Code, Codex (app-server), Elyra/Pi (RPC) and ACP agents;
│                   Claude Code and Codex history import
├── elyra-git       git and gh CLI wrappers: status, diffs, staging, branches, sync,
│                   worktrees, checkpoints, pull requests, review inbox
├── elyra-terminal  PTY and VT emulation (alacritty_terminal): snapshots, keys, mouse,
│                   selection, scrollback search
├── elyra-mcp       minimal MCP server (JSON-RPC over local HTTP) and stdio bridge
└── elyra-app       the GPUI application (binary `elyra`)
```

Main modules of `elyra-app`:

| Module | Responsibility |
| --- | --- |
| `app_state` | Store, project and thread lists, live sessions, fork, handoff, import |
| `thread_session` | One thread's agent process, transcript, queue, goals |
| `workspace` | Window layout, tabs, panels, actions |
| `thread_view`, `transcript`, `composer` | The conversation and the message box |
| `sidebar`, `dialogs`, `palette`, `shortcuts` | Navigation and dialogs |
| `changes_view`, `pr_view`, `review_inbox` | Git, pull requests, code review |
| `files_view`, `search`, `editors` | Files tab, search, external editors |
| `terminal_view` | Terminal rendering, input and panel |
| `context_view` | Notes, pinned messages, recap, instructions, servers, goal |
| `gateway` | Agent gateway tools on top of `elyra-mcp` |
| `automations`, `tasks`, `stats`, `export` | Automations, task board, statistics, ZIP export |
| `preferences`, `themes`, `settings_window` | Settings |
| `updates`, `updater` | Self-update: download, verify, stage, install |
| `lifecycle`, `onboarding`, `about`, `app_icon` | Crash log, single instance, first run, About, Dock |

Schema changes are new entries at the end of `MIGRATIONS` in
`elyra-core/src/store.rs`. Never edit a released migration.

## Releasing

Releases are built by GitHub Actions ([`.github/workflows/release.yml`](../.github/workflows/release.yml))
when a version tag is pushed:

1. Bump `version` in `Cargo.toml` and add a `## <version>` section to
   [CHANGELOG.md](../CHANGELOG.md). Commit and push.
2. Tag and push:
   ```console
   git tag -a v0.1.4 -m "Elyra Workspace 0.1.4" && git push origin v0.1.4
   ```

The workflow:

1. checks that the tag matches `Cargo.toml` and that the changelog has the section
2. runs the tests on an Apple Silicon runner
3. builds the app, signs it with the Developer ID, and notarizes and staples the
   app and the DMG
4. publishes the GitHub release with the DMG, its SHA-256 and the changelog section
   as notes

Installed copies from 0.1.2 on update themselves. **Run workflow** builds and
notarizes without publishing; the DMG is attached to the run.

The in-app updater looks for exactly these release assets:
`Elyra-Workspace-<version>-arm64.dmg` and `Elyra-Workspace-<version>-arm64.dmg.sha256`.
Keep the names.

### Secrets

| Secret | Value |
| --- | --- |
| `MACOS_CERTIFICATE_P12` | The *Developer ID Application* certificate **with its private key**, exported from Keychain Access (My Certificates) as `.p12`, base64-encoded |
| `MACOS_CERTIFICATE_PASSWORD` | The password chosen when exporting the `.p12` |
| `NOTARY_APPLE_ID` | The Apple ID used for notarization |
| `NOTARY_PASSWORD` | An app-specific password for that Apple ID |

```console
base64 -i ~/Desktop/DeveloperID.p12 | gh secret set MACOS_CERTIFICATE_P12 --repo kwhorne/elyra-workspace
gh secret set MACOS_CERTIFICATE_PASSWORD --repo kwhorne/elyra-workspace
gh secret set NOTARY_APPLE_ID --repo kwhorne/elyra-workspace
gh secret set NOTARY_PASSWORD --repo kwhorne/elyra-workspace
```

Check with `gh secret list --repo kwhorne/elyra-workspace`, and delete the exported
`.p12` afterwards.

### Releasing from a Mac

```console
xcrun notarytool store-credentials elyra-workspace --apple-id <apple id> --team-id 7G383N3VY7   # once
scripts/release-macos.sh       # -> target/release/dist/Elyra-Workspace-<version>-arm64.dmg
```

`SKIP_NOTARIZE=1` signs without notarizing. In CI the script takes
`NOTARY_APPLE_ID`, `NOTARY_PASSWORD` and `NOTARY_TEAM_ID` instead of a keychain
profile.

## App icon

The icon is Elyra Conductor's Lyra constellation recolored to Elyra yellow
(`#fbd22d`). Regenerate `assets/icon/` from the Conductor icon with
`python3 scripts/recolor-icon.py [path/to/icon.icns]` (needs Pillow).

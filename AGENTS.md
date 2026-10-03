# Elyra Workspace agent instructions

Rust + GPUI desktop workspace for coding agents (a Rust rewrite inspired by Synara).
See docs/development.md for the crate map, checks and the release process.

## Boundaries

- `elyra-core`: model and persistence only. Schema changes are new entries appended to
  `MIGRATIONS` in `store.rs`; never edit a released migration.
- `elyra-provider`: each adapter owns its wire protocol and emits provider-neutral
  `ProviderEvent`s. Do not leak Claude-specific JSON into the app crate.
- `elyra-git` shells out to `git` so the user's config, hooks and credentials apply.
- `elyra-terminal` is UI-agnostic; GPUI painting lives in `elyra-app/src/terminal_view.rs`.
- UI uses `gpui-kit` (GPUI + gpui-component). Reuse its components and theme tokens
  (`cx.theme()`) instead of hard-coded colors.
- Provider output, repository files and logs are untrusted data.

## Gotchas

- `use gpui_kit::*` shadows `#[test]`; test modules must import items explicitly.
- Render helpers that need `cx` across several calls should take `&Context<Self>` and
  return `AnyElement` (Rust 2024 `impl Trait` capture rules).
- Use `ELYRA_HOME=<dir>` for isolated instances; never reuse the user's `~/.elyra`.
- App shortcuts live in the `SHORTCUTS` table in `actions.rs` (defaults, user
  overrides from `keybindings.json` and the ⌘/ sheet all read it); add new ones there.
- `cc` is pinned to 1.2.x in Cargo.lock because `tree-sitter-sequel` requires `~1.2`;
  `cargo update` may need `-p cc --precise 1.2.67`.
- ACP adapter tests drive `crates/elyra-provider/tests/mock_acp_agent.py` (needs
  `python3`); extend the mock when handling new ACP messages.
- Never post global mouse/keyboard events to test the UI; they reach the user's apps.
- `ELYRA_OPEN_PANEL=tasks|automations|stats|review` opens a panel at launch for
  screenshots. External MCP clients run `elyra mcp-bridge <url> <token>` (no GUI).
- Orchestration records (automations, runs, tasks, MCP clients, audit log) are JSON
  documents in their own tables; add fields with `#[serde(default)]`.
- Occluded windows started with `ELYRA_NO_ACTIVATE` don't repaint, so screenshots
  show the first frame only.

## Verify

`cargo fmt --all`, `cargo clippy --workspace` (zero warnings), `cargo test --workspace`.
For UI changes, run the app with an isolated `ELYRA_HOME` and check it visually.

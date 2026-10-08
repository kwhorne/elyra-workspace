//! App actions, their default shortcuts and user overrides from
//! `~/.elyra/keybindings.json`.

use gpui_kit::{Action, App, KeyBinding, KeyBindingContextPredicate};
use std::collections::HashMap;
use std::rc::Rc;

gpui_kit::actions!(
    elyra,
    [
        Quit,
        NewThread,
        NewChat,
        CloseTab,
        AddProject,
        ToggleSidebar,
        ToggleRightPanel,
        ShowTerminal,
        ShowChanges,
        ToggleTheme,
        Interrupt,
        OpenSettings,
        About,
        FindInThread,
        FocusComposer,
        CheckForUpdates,
        GoBackVersion,
        WhileAway,
        CommitAndPush,
        ShowCodeReview,
        ManageWorktrees,
        CommandPalette,
        FindFile,
        SearchInFiles,
        ShowFiles,
        OpenFilesEditor,
        OpenInEditor,
        ToggleTerminalWorkspace,
        ToggleSplit,
        NewSideChat,
        ShowContext,
        ShowShortcuts,
        NextTab,
        PreviousTab,
        RecentThread,
        GoBack,
        GoForward,
        SelectTab1,
        SelectTab2,
        SelectTab3,
        SelectTab4,
        SelectTab5,
        SelectTab6,
        SelectTab7,
        SelectTab8,
        SelectTab9,
        SplitTerminal,
        SaveFile,
        ImportThreads,
        ForkThread,
        ShowAutomations,
        ShowTasks,
        ShowStats,
        ExportThread,
        OpenDocumentation,
        ShowBrowser,
        BestOfN,
        TodaysWork,
    ]
);

pub struct Shortcut {
    /// Stable id used in keybindings.json.
    pub id: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    pub keys: &'static str,
    pub context: Option<&'static str>,
    pub action: fn() -> Box<dyn Action>,
}

macro_rules! shortcut {
    ($id:literal, $group:literal, $label:literal, $keys:literal, $context:expr, $action:expr) => {
        Shortcut {
            id: $id,
            label: $label,
            group: $group,
            keys: $keys,
            context: $context,
            action: || Box::new($action),
        }
    };
}

const WS: Option<&str> = Some("Workspace");

pub const SHORTCUTS: &[Shortcut] = &[
    shortcut!(
        "command_palette",
        "General",
        "Command palette",
        "cmd-k",
        WS,
        CommandPalette
    ),
    shortcut!(
        "settings",
        "General",
        "Settings",
        "cmd-,",
        None,
        OpenSettings
    ),
    shortcut!(
        "shortcuts",
        "General",
        "Keyboard shortcuts",
        "cmd-/",
        WS,
        ShowShortcuts
    ),
    shortcut!(
        "documentation",
        "General",
        "Documentation",
        "",
        None,
        OpenDocumentation
    ),
    shortcut!(
        "toggle_theme",
        "General",
        "Toggle light/dark",
        "cmd-shift-t",
        WS,
        ToggleTheme
    ),
    shortcut!("quit", "General", "Quit", "cmd-q", None, Quit),
    shortcut!(
        "new_thread",
        "Threads",
        "New thread",
        "cmd-n",
        WS,
        NewThread
    ),
    shortcut!(
        "new_chat",
        "Threads",
        "New chat without a project",
        "cmd-alt-n",
        WS,
        NewChat
    ),
    shortcut!("close_tab", "Threads", "Close tab", "cmd-w", WS, CloseTab),
    shortcut!(
        "next_tab",
        "Threads",
        "Next tab",
        "cmd-shift-]",
        WS,
        NextTab
    ),
    shortcut!(
        "previous_tab",
        "Threads",
        "Previous tab",
        "cmd-shift-[",
        WS,
        PreviousTab
    ),
    shortcut!(
        "recent_thread",
        "Threads",
        "Most recent thread",
        "ctrl-tab",
        WS,
        RecentThread
    ),
    shortcut!("go_back", "Threads", "Back", "cmd-[", WS, GoBack),
    shortcut!("go_forward", "Threads", "Forward", "cmd-]", WS, GoForward),
    shortcut!(
        "select_tab_1",
        "Threads",
        "Go to tab 1…9",
        "cmd-1",
        WS,
        SelectTab1
    ),
    shortcut!("select_tab_2", "Threads", "", "cmd-2", WS, SelectTab2),
    shortcut!("select_tab_3", "Threads", "", "cmd-3", WS, SelectTab3),
    shortcut!("select_tab_4", "Threads", "", "cmd-4", WS, SelectTab4),
    shortcut!("select_tab_5", "Threads", "", "cmd-5", WS, SelectTab5),
    shortcut!("select_tab_6", "Threads", "", "cmd-6", WS, SelectTab6),
    shortcut!("select_tab_7", "Threads", "", "cmd-7", WS, SelectTab7),
    shortcut!("select_tab_8", "Threads", "", "cmd-8", WS, SelectTab8),
    shortcut!("select_tab_9", "Threads", "", "cmd-9", WS, SelectTab9),
    shortcut!("split", "Threads", "Split chat", "cmd-\\", WS, ToggleSplit),
    shortcut!(
        "side_chat",
        "Threads",
        "New side chat",
        "cmd-alt-s",
        WS,
        NewSideChat
    ),
    shortcut!(
        "fork_thread",
        "Threads",
        "Fork thread",
        "cmd-shift-k",
        WS,
        ForkThread
    ),
    shortcut!(
        "import_threads",
        "Threads",
        "Import sessions (Claude Code, Codex)",
        "cmd-i",
        WS,
        ImportThreads
    ),
    shortcut!(
        "export_thread",
        "Threads",
        "Export thread",
        "",
        WS,
        ExportThread
    ),
    shortcut!("tasks", "Panels", "Task board", "cmd-alt-t", WS, ShowTasks),
    shortcut!(
        "automations",
        "Panels",
        "Automations",
        "cmd-alt-a",
        WS,
        ShowAutomations
    ),
    shortcut!("stats", "Panels", "Usage statistics", "", WS, ShowStats),
    shortcut!("best_of_n", "Threads", "Best of N", "", WS, BestOfN),
    shortcut!("todays_work", "Panels", "Today's work", "", WS, TodaysWork),
    shortcut!(
        "focus_composer",
        "Threads",
        "Focus composer",
        "cmd-l",
        WS,
        FocusComposer
    ),
    shortcut!(
        "find_in_thread",
        "Threads",
        "Find in thread",
        "cmd-f",
        Some("Composer"),
        FindInThread
    ),
    shortcut!(
        "interrupt",
        "Threads",
        "Stop the agent",
        "ctrl-c",
        Some("Composer"),
        Interrupt
    ),
    shortcut!(
        "code_review",
        "Threads",
        "Code review inbox",
        "cmd-shift-r",
        WS,
        ShowCodeReview
    ),
    shortcut!(
        "add_project",
        "Projects",
        "Add project",
        "cmd-shift-o",
        WS,
        AddProject
    ),
    shortcut!(
        "open_in_editor",
        "Projects",
        "Open in external editor",
        "cmd-o",
        WS,
        OpenInEditor
    ),
    shortcut!("find_file", "Projects", "Find file", "cmd-p", WS, FindFile),
    shortcut!(
        "search_in_files",
        "Projects",
        "Search in files",
        "cmd-shift-f",
        WS,
        SearchInFiles
    ),
    shortcut!(
        "save_file",
        "Projects",
        "Save file",
        "cmd-s",
        Some("FilesView"),
        SaveFile
    ),
    shortcut!(
        "toggle_sidebar",
        "Panels",
        "Toggle sidebar",
        "cmd-b",
        WS,
        ToggleSidebar
    ),
    shortcut!(
        "toggle_right",
        "Panels",
        "Toggle tools panel",
        "cmd-alt-b",
        WS,
        ToggleRightPanel
    ),
    shortcut!(
        "changes",
        "Panels",
        "Changes",
        "cmd-shift-g",
        WS,
        ShowChanges
    ),
    shortcut!("files", "Panels", "Files", "cmd-shift-e", WS, ShowFiles),
    shortcut!(
        "files_editor",
        "Panels",
        "Large file editor",
        "cmd-alt-e",
        WS,
        OpenFilesEditor
    ),
    shortcut!(
        "browser",
        "Panels",
        "Browser",
        "cmd-shift-b",
        WS,
        ShowBrowser
    ),
    shortcut!(
        "context",
        "Panels",
        "Context and notes",
        "cmd-shift-i",
        WS,
        ShowContext
    ),
    shortcut!(
        "terminal",
        "Panels",
        "Toggle terminal",
        "cmd-j",
        WS,
        ShowTerminal
    ),
    shortcut!(
        "terminal_workspace",
        "Panels",
        "Full-width terminal",
        "cmd-shift-j",
        WS,
        ToggleTerminalWorkspace
    ),
    shortcut!(
        "split_terminal",
        "Panels",
        "Split terminal",
        "cmd-d",
        Some("TerminalPanel"),
        SplitTerminal
    ),
    shortcut!(
        "commit_and_push",
        "Git",
        "Commit and push",
        "ctrl-cmd-p",
        WS,
        CommitAndPush
    ),
];

pub fn keybindings_path() -> std::path::PathBuf {
    elyra_core::paths::data_dir().join("keybindings.json")
}

/// User overrides: `{ "find_file": "cmd-shift-p", "terminal": null }`.
/// A null or empty value removes the shortcut.
pub fn user_overrides() -> HashMap<String, Option<String>> {
    std::fs::read_to_string(keybindings_path())
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

/// The keys in effect for each shortcut, after user overrides.
pub fn effective_keys() -> Vec<(&'static Shortcut, Option<String>)> {
    let overrides = user_overrides();
    SHORTCUTS
        .iter()
        .map(|shortcut| {
            let keys = match overrides.get(shortcut.id) {
                Some(value) => value.clone().filter(|v| !v.trim().is_empty()),
                None => Some(shortcut.keys.to_string()),
            };
            (shortcut, keys)
        })
        .collect()
}

pub fn bind_keys(cx: &mut App) {
    let mut bindings = Vec::new();
    for (shortcut, keys) in effective_keys() {
        let Some(keys) = keys.filter(|k| !k.is_empty()) else {
            continue;
        };
        let context = shortcut
            .context
            .and_then(|c| KeyBindingContextPredicate::parse(c).ok())
            .map(Rc::new);
        match KeyBinding::load(
            &keys,
            (shortcut.action)(),
            context,
            false,
            None,
            cx.keyboard_mapper().as_ref(),
        ) {
            Ok(binding) => bindings.push(binding),
            Err(err) => log::warn!("invalid shortcut {keys:?} for {}: {err:#}", shortcut.id),
        }
    }
    cx.bind_keys(bindings);
}

/// "cmd-shift-o" → "⇧⌘O".
pub fn display_keys(keys: &str) -> String {
    keys.split(' ')
        .map(|stroke| {
            let mut mods = String::new();
            let mut key = String::new();
            for part in stroke.split('-').filter(|p| !p.is_empty()) {
                match part {
                    "ctrl" => mods.push('⌃'),
                    "alt" => mods.push('⌥'),
                    "shift" => mods.push('⇧'),
                    "cmd" => mods.push('⌘'),
                    "enter" => key.push('↩'),
                    "tab" => key.push_str("Tab"),
                    "escape" => key.push_str("Esc"),
                    other => key.push_str(&other.to_uppercase()),
                }
            }
            if stroke.ends_with("--") {
                key.push('-');
            }
            // Order modifiers the macOS way: ⌃⌥⇧⌘.
            let order = ['⌃', '⌥', '⇧', '⌘'];
            let mods: String = order.iter().filter(|m| mods.contains(**m)).collect();
            format!("{mods}{key}")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::{SHORTCUTS, display_keys};
    use std::collections::HashSet;

    #[test]
    fn shortcut_ids_are_unique_and_keys_display() {
        let ids: HashSet<_> = SHORTCUTS.iter().map(|s| s.id).collect();
        assert_eq!(ids.len(), SHORTCUTS.len());
        assert_eq!(display_keys("cmd-shift-o"), "⇧⌘O");
        assert_eq!(display_keys("ctrl-cmd-p"), "⌃⌘P");
        assert_eq!(display_keys("cmd-\\"), "⌘\\");
    }
}

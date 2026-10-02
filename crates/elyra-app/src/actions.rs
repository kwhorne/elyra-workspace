use gpui_kit::{App, KeyBinding};

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
    ]
);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("cmd-n", NewThread, Some("Workspace")),
        KeyBinding::new("cmd-alt-n", NewChat, Some("Workspace")),
        KeyBinding::new("cmd-w", CloseTab, Some("Workspace")),
        KeyBinding::new("cmd-o", AddProject, Some("Workspace")),
        KeyBinding::new("cmd-b", ToggleSidebar, Some("Workspace")),
        KeyBinding::new("cmd-alt-b", ToggleRightPanel, Some("Workspace")),
        KeyBinding::new("cmd-j", ShowTerminal, Some("Workspace")),
        KeyBinding::new("cmd-shift-g", ShowChanges, Some("Workspace")),
        KeyBinding::new("cmd-shift-t", ToggleTheme, Some("Workspace")),
        KeyBinding::new("ctrl-c", Interrupt, Some("Composer")),
        KeyBinding::new("cmd-f", FindInThread, Some("Composer")),
        KeyBinding::new("cmd-l", FocusComposer, Some("Workspace")),
    ]);
}

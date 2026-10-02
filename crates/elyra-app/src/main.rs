mod about;
mod actions;
mod app_icon;
mod app_state;
mod changes_view;
mod composer;
mod context_view;
mod dialogs;
mod editors;
mod files_view;
mod lifecycle;
mod onboarding;
mod palette;
mod pr_view;
mod preferences;
mod review_inbox;
mod search;
mod settings_window;
mod shortcuts;
mod sidebar;
mod terminal_view;
mod themes;
mod thread_session;
mod thread_view;
mod transcript;
mod updates;
mod workspace;

use app_state::AppState;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{TitleBar, WindowExt as _};
use gpui_kit::*;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    lifecycle::install_crash_log();
    let Some(_instance_lock) = lifecycle::acquire_instance_lock() else {
        eprintln!(
            "Elyra Workspace is already running with data directory {}",
            elyra_core::paths::data_dir().display()
        );
        std::process::exit(0);
    };
    elyra_core::shell_env::inherit_login_shell_path();

    let state = match AppState::load() {
        Ok(state) => state,
        Err(err) => {
            eprintln!("Elyra Workspace could not open its database: {err:#}");
            std::process::exit(1);
        }
    };

    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            actions::bind_keys(cx);
            terminal_view::bind_keys(cx);
            themes::register(cx);
            app_icon::install_dock_icon();

            let app = cx.new(|_| state);
            preferences::init(&app, cx);
            cx.set_menus(menus());

            let mut options = TitleBar::window_options();
            options.window_bounds = Some(saved_bounds(&app, cx));
            options.window_min_size = Some(size(px(820.), px(520.)));
            // Development: open in the background without taking focus.
            let background = std::env::var_os("ELYRA_NO_ACTIVATE").is_some();
            options.focus = !background;
            let workspace_app = app.clone();
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| workspace::Workspace::new(workspace_app, window, cx))
            })
            .expect("failed to open the Elyra Workspace window");

            let quit_app = app.clone();
            cx.on_action(move |_: &actions::Quit, cx| request_quit(&quit_app, cx));
            cx.on_action(|_: &actions::OpenSettings, cx| settings_window::open(cx));
            cx.on_action(|_: &actions::CheckForUpdates, cx| check_for_updates(true, cx));
            let shutdown_app = app.clone();
            cx.on_app_quit(move |cx| {
                shutdown_app.update(cx, |app, cx| app.prepare_quit(cx));
                async {}
            })
            .detach();
            if !background {
                cx.activate(true);
            }
            if preferences::Preferences::global(cx).check_updates {
                check_for_updates(false, cx);
            }
        });
}

fn menus() -> Vec<Menu> {
    vec![
        Menu {
            name: "Elyra Workspace".into(),
            items: vec![
                MenuItem::action("About Elyra Workspace", actions::About),
                MenuItem::action("Check for Updates…", actions::CheckForUpdates),
                MenuItem::separator(),
                MenuItem::action("Settings…", actions::OpenSettings),
                MenuItem::separator(),
                MenuItem::action("Quit Elyra Workspace", actions::Quit),
            ],
            disabled: false,
        },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("New Thread", actions::NewThread),
                MenuItem::action("New Chat", actions::NewChat),
                MenuItem::action("New Side Chat", actions::NewSideChat),
                MenuItem::action("Add Project…", actions::AddProject),
                MenuItem::action("Open in Editor", actions::OpenInEditor),
                MenuItem::separator(),
                MenuItem::action("Find File…", actions::FindFile),
                MenuItem::action("Search in Files…", actions::SearchInFiles),
                MenuItem::separator(),
                MenuItem::action("Manage Worktrees…", actions::ManageWorktrees),
                MenuItem::separator(),
                MenuItem::action("Close Tab", actions::CloseTab),
            ],
            disabled: false,
        },
        Menu {
            name: "View".into(),
            items: vec![
                MenuItem::action("Command Palette…", actions::CommandPalette),
                MenuItem::separator(),
                MenuItem::action("Toggle Sidebar", actions::ToggleSidebar),
                MenuItem::action("Toggle Tools Panel", actions::ToggleRightPanel),
                MenuItem::action("Terminal", actions::ShowTerminal),
                MenuItem::action("Full-Width Terminal", actions::ToggleTerminalWorkspace),
                MenuItem::action("Changes", actions::ShowChanges),
                MenuItem::action("Files", actions::ShowFiles),
                MenuItem::action("Context and Notes", actions::ShowContext),
                MenuItem::action("Code Review", actions::ShowCodeReview),
                MenuItem::separator(),
                MenuItem::action("Split Chat", actions::ToggleSplit),
                MenuItem::action("Back", actions::GoBack),
                MenuItem::action("Forward", actions::GoForward),
                MenuItem::separator(),
                MenuItem::action("Toggle Light/Dark", actions::ToggleTheme),
            ],
            disabled: false,
        },
        Menu {
            name: "Help".into(),
            items: vec![MenuItem::action(
                "Keyboard Shortcuts",
                actions::ShowShortcuts,
            )],
            disabled: false,
        },
    ]
}

/// The main window's last bounds, or a centred default.
fn saved_bounds(app: &Entity<AppState>, cx: &App) -> WindowBounds {
    let saved = app
        .read(cx)
        .store
        .setting("window_bounds")
        .ok()
        .flatten()
        .and_then(|value| {
            let parts: Vec<f32> = value.split(',').filter_map(|p| p.parse().ok()).collect();
            (parts.len() == 4 && parts[2] >= 600. && parts[3] >= 400.).then(|| {
                Bounds::new(
                    point(px(parts[0]), px(parts[1])),
                    size(px(parts[2]), px(parts[3])),
                )
            })
        });
    match saved {
        Some(bounds) => WindowBounds::Windowed(bounds),
        None => WindowBounds::centered(size(px(1360.), px(860.)), cx),
    }
}

/// Quit, asking first when agents are still working.
fn request_quit(app: &Entity<AppState>, cx: &mut App) {
    let running = app.read(cx).running_threads(cx);
    if running.is_empty() {
        cx.quit();
        return;
    }
    let Some(window) = cx.active_window().or_else(|| cx.windows().first().copied()) else {
        cx.quit();
        return;
    };
    let names = running
        .iter()
        .take(5)
        .map(|t| format!("• {}", t.title))
        .collect::<Vec<_>>()
        .join("\n");
    let count = running.len();
    let app = app.clone();
    let _ = window.update(cx, move |_, window, cx| {
        window.open_alert_dialog(cx, move |alert, _, _| {
            let app = app.clone();
            alert
                .title(if count == 1 {
                    "An agent is still working".to_string()
                } else {
                    format!("{count} agents are still working")
                })
                .description(format!(
                    "{names}\n\nQuitting stops them. You can resume each thread after the next launch."
                ))
                .confirm()
                .ok_text("Quit")
                .on_ok(move |_, _, cx| {
                    app.update(cx, |app, cx| app.prepare_quit(cx));
                    cx.quit();
                    true
                })
        });
    });
}

/// Look for a newer release. `manual` also reports "up to date" and errors.
fn check_for_updates(manual: bool, cx: &mut App) {
    let job = cx
        .background_executor()
        .spawn(async { updates::latest_release() });
    cx.spawn(async move |cx| {
        let result = job.await;
        cx.update(|cx| {
            let note = match result {
                Ok(release) if updates::is_newer(&release.version, env!("CARGO_PKG_VERSION")) => {
                    let url = release.url.clone();
                    Some(
                        Notification::info(format!(
                            "Elyra Workspace {} is available (you have {}). Click to open the release.",
                            release.version,
                            env!("CARGO_PKG_VERSION")
                        ))
                        .autohide(false)
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                    )
                }
                Ok(_) if manual => Some(Notification::success(format!(
                    "Elyra Workspace {} is up to date.",
                    env!("CARGO_PKG_VERSION")
                ))),
                Err(err) if manual => Some(Notification::warning(format!(
                    "Could not check for updates: {err:#}"
                ))),
                _ => None,
            };
            if let Some(note) = note
                && let Some(window) = cx.windows().first().copied()
            {
                let _ = window.update(cx, |_, window, cx| window.push_notification(note, cx));
            }
        });
    })
    .detach();
}

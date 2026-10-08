mod about;
mod actions;
mod app_icon;
mod app_state;
mod automations;
mod browser_tools;
mod browser_view;
mod changes_view;
mod checks;
mod composer;
mod context_view;
mod conventional;
mod day_summary;
mod dialogs;
mod editors;
mod export;
mod felagi;
mod felagi_board;
mod felagi_report;
mod felagi_runtime;
mod files_view;
mod gateway;
mod grove;
mod lifecycle;
mod onboarding;
mod palette;
mod pr_view;
mod preferences;
mod quitting;
mod race;
mod recovery;
mod review_inbox;
mod search;
mod settings_window;
mod shared_setup;
mod shortcuts;
mod sidebar;
mod stats;
mod tasks;
mod terminal_view;
mod themes;
mod thread_access;
mod thread_session;
mod thread_view;
mod transcript;
mod updater;
mod updates;
mod webview;
mod workspace;

use app_state::AppState;
use gpui_kit::component::TitleBar;
use gpui_kit::*;

fn main() {
    // `elyra mcp-bridge <url> <token>`: relay MCP over stdio for external
    // clients (Claude Desktop, Codex). No window, no database.
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("mcp-bridge") {
        let (Some(url), Some(token)) = (args.get(2), args.get(3)) else {
            eprintln!("usage: elyra mcp-bridge <url> <token>");
            std::process::exit(2);
        };
        if let Err(err) = elyra_mcp::run_bridge(url, token) {
            eprintln!("elyra mcp-bridge: {err:#}");
            std::process::exit(1);
        }
        return;
    }
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    lifecycle::install_crash_log();
    let Some(_instance_lock) = lifecycle::acquire_instance_lock() else {
        eprintln!(
            "Elyra Workspace is already running with data directory {}",
            elyra_core::paths::data_dir().display()
        );
        std::process::exit(0);
    };
    // An update that keeps failing to start: the previous version took over.
    if std::env::var_os("ELYRA_HEADLESS").is_none() && recovery::on_launch() {
        std::process::exit(0);
    }
    elyra_core::shell_env::inherit_login_shell_path();

    let state = match AppState::load() {
        Ok(state) => state,
        Err(err) => {
            eprintln!("Elyra Workspace could not open its database: {err:#}");
            std::process::exit(1);
        }
    };

    let application = gpui_kit::application().with_assets(gpui_kit::assets::AllAssets);
    // The Dock icon brings the window back after it was closed or hidden.
    application.on_reopen(quitting::reopen);
    application.run(move |cx| {
        gpui_kit::init(cx);
        actions::bind_keys(cx);
        terminal_view::bind_keys(cx);
        themes::register(cx);
        // Tests: no window, Dock icon or update checks; drive it through the
        // agent gateway (scripts/scenarios).
        let headless = std::env::var_os("ELYRA_HEADLESS").is_some();
        if headless {
            app_icon::hide_from_dock();
        } else {
            app_icon::install_dock_icon();
        }

        let app = cx.new(|_| state);
        preferences::init(&app, cx);
        gateway::init(app.clone(), cx);
        felagi_runtime::init(app.clone(), cx);
        // Learn Grove's sites early, so agents can be given its tools.
        cx.background_executor()
            .spawn(async { grove::sites() })
            .detach();
        automations::init(app.clone(), cx);
        cx.set_menus(menus());

        // Development: open in the background without taking focus.
        let background = headless || std::env::var_os("ELYRA_NO_ACTIVATE").is_some();
        if !headless {
            open_main_window(app.clone(), !background, cx);
        }

        quitting::init(app.clone(), cx);
        if !headless {
            quitting::offer_resume(cx);
            recovery::announce(cx);
            recovery::confirm_start_later(cx);
        }
        cx.on_action(|_: &actions::Quit, cx| quitting::request(cx));
        cx.on_action(|_: &actions::OpenSettings, cx| settings_window::open(cx));
        cx.on_action(|_: &actions::CheckForUpdates, cx| updater::check(true, cx));
        cx.on_action(|_: &actions::GoBackVersion, cx| recovery::offer_go_back(cx));
        cx.on_action(|_: &actions::OpenDocumentation, cx| cx.open_url(DOCS_URL));
        let shutdown_app = app.clone();
        cx.on_app_quit(move |cx| {
            if !headless {
                recovery::confirm_start();
            }
            felagi_runtime::hand_back(cx);
            shutdown_app.update(cx, |app, cx| app.prepare_quit(cx));
            updater::on_quit(cx);
            async {}
        })
        .detach();
        if !background {
            cx.activate(true);
        }
        if !headless {
            updater::init(app.clone(), cx);
        }
    });
}

/// The user guide.
pub(crate) const DOCS_URL: &str = "https://elyracode.com/docs/workspace";

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
                MenuItem::action("Fork Thread", actions::ForkThread),
                MenuItem::action("Import Sessions…", actions::ImportThreads),
                MenuItem::action("Export Thread…", actions::ExportThread),
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
                MenuItem::action("Browser", actions::ShowBrowser),
                MenuItem::action("Context and Notes", actions::ShowContext),
                MenuItem::action("Code Review", actions::ShowCodeReview),
                MenuItem::action("Task Board", actions::ShowTasks),
                MenuItem::action("Automations", actions::ShowAutomations),
                MenuItem::action("Usage Statistics", actions::ShowStats),
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
            items: vec![
                MenuItem::action("Elyra Workspace Documentation", actions::OpenDocumentation),
                MenuItem::action("Keyboard Shortcuts", actions::ShowShortcuts),
            ],
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

/// Open the workspace window, where it was last time.
pub(crate) fn open_main_window(app: Entity<AppState>, focus: bool, cx: &mut App) {
    let mut options = TitleBar::window_options();
    options.window_bounds = Some(saved_bounds(&app, cx));
    options.window_min_size = Some(size(px(820.), px(520.)));
    options.focus = focus;
    if let Err(err) = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| workspace::Workspace::new(app, window, cx))
    }) {
        log::error!("opening the Elyra Workspace window: {err:#}");
    }
}

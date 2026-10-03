//! The Settings window (⌘,): appearance, fonts and terminal behavior.

use crate::preferences::{self, Preferences, ProviderSettings};
use crate::themes;
use elyra_core::ProviderKind;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::h_flex;
use gpui_kit::component::setting::{
    NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings,
};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, TitleBar, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

struct SettingsWindow(Option<AnyWindowHandle>);

impl Global for SettingsWindow {}

/// CLI versions found for each provider (`None` = not found), filled in the
/// background when Settings opens.
#[derive(Default)]
struct ProviderStatus(std::collections::HashMap<ProviderKind, Option<String>>);

impl Global for ProviderStatus {}

fn refresh_provider_status(cx: &mut App) {
    let launches: Vec<(ProviderKind, Option<std::path::PathBuf>)> = ProviderKind::ALL
        .into_iter()
        .map(|kind| {
            let path = Preferences::global(cx)
                .launch(kind, None)
                .executable
                .or_else(|| elyra_provider::find_executable(kind));
            (kind, path)
        })
        .collect();
    let job = cx.background_executor().spawn(async move {
        launches
            .into_iter()
            .map(|(kind, path)| {
                let version = path.map(|path| {
                    elyra_provider::cli_version(&path).unwrap_or_else(|| path.display().to_string())
                });
                (kind, version)
            })
            .collect::<std::collections::HashMap<_, _>>()
    });
    cx.spawn(async move |cx| {
        let status = job.await;
        cx.update(|cx| {
            cx.set_global(ProviderStatus(status));
            cx.refresh_windows();
        });
    })
    .detach();
}

/// Open the provider's sign-in command in Terminal, with its environment.
fn sign_in(kind: ProviderKind, cx: &mut App) {
    let Some(command) = elyra_provider::login_command(kind) else {
        return;
    };
    let launch = Preferences::global(cx).launch(kind, None);
    let command = match &launch.executable {
        Some(path) => {
            let rest = command.split_once(' ').map(|(_, rest)| rest).unwrap_or("");
            format!("{} {rest}", shell_quote(&path.display().to_string()))
        }
        None => command.to_string(),
    };
    let mut script = String::from("#!/bin/zsh -l\n");
    for (key, value) in &launch.env {
        script.push_str(&format!("export {key}={}\n", shell_quote(value)));
    }
    script.push_str(&format!(
        "echo 'Signing in to {}…'\n{command}\n",
        kind.label()
    ));
    let path = std::env::temp_dir().join(format!("elyra-sign-in-{}.command", kind.as_str()));
    let result = std::fs::write(&path, script).and_then(|()| {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
    });
    match result {
        Ok(()) => cx.open_with_system(&path),
        Err(err) => log::error!("sign-in script: {err}"),
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Open the Settings window, or bring the existing one to the front.
pub fn open(cx: &mut App) {
    if let Some(handle) = cx.try_global::<SettingsWindow>().and_then(|w| w.0)
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    let mut options = TitleBar::window_options();
    options.window_bounds = Some(WindowBounds::centered(size(px(860.), px(640.)), cx));
    options.window_min_size = Some(size(px(640.), px(420.)));
    refresh_provider_status(cx);
    match gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| SettingsView)) {
        Ok((handle, _)) => cx.set_global(SettingsWindow(Some(handle))),
        Err(err) => log::error!("opening settings: {err:#}"),
    }
}

struct SettingsView;

fn options(
    values: impl IntoIterator<Item = (String, String)>,
) -> Vec<(SharedString, SharedString)> {
    values
        .into_iter()
        .map(|(value, label)| (value.into(), label.into()))
        .collect()
}

fn same(values: Vec<String>) -> Vec<(SharedString, SharedString)> {
    options(values.into_iter().map(|v| (v.clone(), v)))
}

fn number(min: f64, max: f64, step: f64) -> NumberFieldOptions {
    NumberFieldOptions { min, max, step }
}

fn dropdown(
    choices: Vec<(SharedString, SharedString)>,
    get: impl Fn(&Preferences) -> String + 'static,
    set: impl Fn(&mut Preferences, String) + 'static,
) -> SettingField<SharedString> {
    SettingField::scrollable_dropdown(
        choices,
        move |cx| get(Preferences::global(cx)).into(),
        move |value, cx| preferences::update(cx, |prefs| set(prefs, value.to_string())),
    )
}

fn switch(
    get: impl Fn(&Preferences) -> bool + 'static,
    set: impl Fn(&mut Preferences, bool) + 'static,
) -> SettingField<bool> {
    SettingField::switch(
        move |cx| get(Preferences::global(cx)),
        move |value, cx| preferences::update(cx, |prefs| set(prefs, value)),
    )
}

fn number_field(
    range: NumberFieldOptions,
    get: impl Fn(&Preferences) -> f64 + 'static,
    set: impl Fn(&mut Preferences, f64) + 'static,
) -> SettingField<f64> {
    SettingField::number_input(
        range,
        move |cx| get(Preferences::global(cx)),
        move |value, cx| preferences::update(cx, |prefs| set(prefs, value)),
    )
}

impl SettingsView {
    fn pages(cx: &App) -> Vec<SettingPage> {
        let defaults = Preferences::default();
        let theme_choices = options(
            themes::all_names(cx)
                .into_iter()
                .map(|n| (n.to_string(), n.to_string())),
        );
        let all_themes = themes::all_names(cx);
        let light_choices = same(
            all_themes
                .iter()
                .filter(|n| !themes::is_dark(n))
                .cloned()
                .collect(),
        );
        let dark_choices = same(
            all_themes
                .iter()
                .filter(|n| themes::is_dark(n))
                .cloned()
                .collect(),
        );
        let mut editor_choices = vec![(String::new(), "Automatic".to_string())];
        editor_choices.extend(
            crate::editors::installed()
                .into_iter()
                .map(|e| (e.name.to_string(), e.name.to_string())),
        );
        let mono_fonts = preferences::mono_fonts(cx);
        let mut ui_fonts = vec![(String::new(), "System".to_string())];
        ui_fonts.extend(
            preferences::ui_fonts(cx)
                .into_iter()
                .map(|f| (f.clone(), f)),
        );
        let mut terminal_fonts = vec![(String::new(), "Same as code font".to_string())];
        terminal_fonts.extend(mono_fonts.iter().map(|f| (f.clone(), f.clone())));

        let appearance = SettingPage::new("Appearance")
            .icon(IconName::Palette)
            .resettable(true)
            .group(
                SettingGroup::new()
                    .title("Theme")
                    .item(
                        SettingItem::new(
                            "Color theme",
                            dropdown(theme_choices.clone(), |p| p.theme.clone(), |p, v| {
                                p.follow_system = false;
                                p.theme = v
                            })
                            .default_value(SharedString::from(defaults.theme.clone())),
                        )
                        .description("Applies to the whole window, including the terminal.")
                        .keywords(["tokyo night", "palenight", "dracula", "nord", "dark", "light"]),
                    )
                    .item(
                        SettingItem::new(
                            "Follow system appearance",
                            switch(|p| p.follow_system, |p, v| p.follow_system = v)
                                .default_value(defaults.follow_system),
                        )
                        .description("Switch between the light and dark theme below with macOS."),
                    )
                    .item(SettingItem::new(
                        "Light theme",
                        dropdown(light_choices, |p| p.light_theme.clone(), |p, v| {
                            if !p.follow_system && !themes::is_dark(&p.theme) {
                                p.theme = v.clone();
                            }
                            p.light_theme = v
                        })
                        .default_value(SharedString::from(defaults.light_theme.clone())),
                    ))
                    .item(SettingItem::new(
                        "Dark theme",
                        dropdown(dark_choices, |p| p.dark_theme.clone(), |p, v| {
                            if !p.follow_system && themes::is_dark(&p.theme) {
                                p.theme = v.clone();
                            }
                            p.dark_theme = v
                        })
                        .default_value(SharedString::from(defaults.dark_theme.clone())),
                    ))
                    .item(
                        SettingItem::new(
                            "Custom themes",
                            SettingField::render(|_, _, _| {
                                Button::new("open-themes-folder")
                                    .small()
                                    .label("Open folder")
                                    .on_click(|_, _, cx| {
                                        let dir = themes::custom_dir();
                                        let _ = std::fs::create_dir_all(&dir);
                                        cx.open_with_system(&dir);
                                    })
                            }),
                        )
                        .description(
                            "Put gpui-component theme files (*.json) in ~/.elyra/themes. They load at startup.",
                        ),
                    ),
            )
            .group(
                SettingGroup::new()
                    .title("Layout")
                    .item(
                        SettingItem::new(
                            "Conversation width",
                            number_field(number(0., 2400., 20.), |p| p.chat_width as f64, |p, v| {
                                p.chat_width = v.max(0.).round() as f32
                            })
                            .default_value(defaults.chat_width as f64),
                        )
                        .description("Maximum width of messages in points. 0 uses the full width."),
                    )
                    .item(SettingItem::new(
                        "Density",
                        dropdown(
                            options([
                                ("comfortable".to_string(), "Comfortable".to_string()),
                                ("compact".to_string(), "Compact".to_string()),
                            ]),
                            |p| p.density.clone(),
                            |p, v| p.density = v,
                        )
                        .default_value(SharedString::from(defaults.density.clone())),
                    )),
            )
            .group(
                SettingGroup::new()
                    .title("Interface font")
                    .item(
                        SettingItem::new(
                            "Font family",
                            dropdown(
                                options(ui_fonts),
                                |p| p.ui_font_family.clone(),
                                |p, v| p.ui_font_family = v,
                            )
                            .default_value(SharedString::default()),
                        )
                        .keywords(["font", "typeface"]),
                    )
                    .item(
                        SettingItem::new(
                            "Font size",
                            number_field(
                                number(10., 22., 1.),
                                |p| p.ui_font_size as f64,
                                |p, v| p.ui_font_size = v.round() as f32,
                            )
                            .default_value(defaults.ui_font_size as f64),
                        )
                        .description("Scales the whole interface."),
                    ),
            );

        let code = SettingPage::new("Code")
            .icon(IconName::Code)
            .resettable(true)
            .group(
                SettingGroup::new()
                    .title("Code font")
                    .description("Used for diffs, tool output and code in the transcript.")
                    .item(
                        SettingItem::new(
                            "Font family",
                            dropdown(
                                same(mono_fonts.clone()),
                                |p| p.mono_font_family.clone(),
                                |p, v| p.mono_font_family = v,
                            )
                            .default_value(SharedString::from(defaults.mono_font_family.clone())),
                        )
                        .keywords(["mono", "monaco", "menlo", "font"]),
                    )
                    .item(SettingItem::new(
                        "Font size",
                        number_field(
                            number(9., 24., 1.),
                            |p| p.mono_font_size as f64,
                            |p, v| p.mono_font_size = v.round() as f32,
                        )
                        .default_value(defaults.mono_font_size as f64),
                    )),
            );

        let cursor_shapes = options([
            ("block".to_string(), "Block".to_string()),
            ("beam".to_string(), "Bar".to_string()),
            ("underline".to_string(), "Underline".to_string()),
        ]);
        let terminal = SettingPage::new("Terminal")
            .icon(IconName::SquareTerminal)
            .resettable(true)
            .group(
                SettingGroup::new()
                    .title("Font")
                    .item(SettingItem::new(
                        "Font family",
                        dropdown(options(terminal_fonts), |p| p.terminal_font_family.clone(), |p, v| {
                            p.terminal_font_family = v
                        })
                        .default_value(SharedString::default()),
                    ).keywords(["mono", "font", "nerd"]))
                    .item(SettingItem::new(
                        "Font size",
                        number_field(number(8., 32., 1.), |p| p.terminal_font_size as f64, |p, v| {
                            p.terminal_font_size = v.round() as f32
                        })
                        .default_value(defaults.terminal_font_size as f64),
                    ))
                    .item(SettingItem::new(
                        "Line height",
                        number_field(number(1.0, 2.0, 0.05), |p| p.terminal_line_height as f64, |p, v| {
                            p.terminal_line_height = ((v * 100.).round() / 100.) as f32
                        })
                        .default_value(defaults.terminal_line_height as f64),
                    )),
            )
            .group(
                SettingGroup::new()
                    .title("Cursor")
                    .item(SettingItem::new(
                        "Cursor shape",
                        dropdown(cursor_shapes, |p| p.terminal_cursor_shape.clone(), |p, v| {
                            p.terminal_cursor_shape = v
                        })
                        .default_value(SharedString::from(defaults.terminal_cursor_shape.clone())),
                    ).description("Programs such as vim can still change the shape."))
                    .item(SettingItem::new(
                        "Blinking cursor",
                        switch(|p| p.terminal_cursor_blink, |p, v| p.terminal_cursor_blink = v)
                            .default_value(defaults.terminal_cursor_blink),
                    )),
            )
            .group(
                SettingGroup::new()
                    .title("Keyboard & mouse")
                    .item(SettingItem::new(
                        "Use Option as Meta key",
                        switch(|p| p.terminal_option_as_meta, |p, v| p.terminal_option_as_meta = v)
                            .default_value(defaults.terminal_option_as_meta),
                    ).description(
                        "Sends Esc+key for ⌥-combinations (Emacs, readline). Leave off to type characters such as [ ] { } | @ ~ with ⌥ on Nordic keyboards.",
                    ).keywords(["alt", "meta", "option"]))
                    .item(SettingItem::new(
                        "Copy on select",
                        switch(|p| p.terminal_copy_on_select, |p, v| p.terminal_copy_on_select = v)
                            .default_value(defaults.terminal_copy_on_select),
                    ).description("Copy selected text to the clipboard immediately.")),
            )
            .group(
                SettingGroup::new()
                    .title("Shell")
                    .item(SettingItem::new(
                        "Shell command",
                        SettingField::input(
                            |cx| Preferences::global(cx).terminal_shell.clone().into(),
                            |value, cx| {
                                preferences::update(cx, |p| p.terminal_shell = value.trim().to_string())
                            },
                        )
                        .default_value(SharedString::default()),
                    ).description("Leave empty for your login shell. Applies to new terminals."))
                    .item(SettingItem::new(
                        "Scrollback lines",
                        number_field(number(0., 200_000., 1000.), |p| p.terminal_scrollback as f64, |p, v| {
                            p.terminal_scrollback = v.max(0.) as usize
                        })
                        .default_value(defaults.terminal_scrollback as f64),
                    )),
            );

        let general = SettingPage::new("General")
            .icon(IconName::Settings)
            .resettable(true)
            .group(
                SettingGroup::new().title("External editor").item(
                    SettingItem::new(
                        "Open projects in",
                        dropdown(
                            options(editor_choices),
                            |p| p.editor.clone(),
                            |p, v| p.editor = v,
                        )
                        .default_value(SharedString::default()),
                    )
                    .description("Used by ⌘O and the Open button in the title bar.")
                    .keywords(["vscode", "cursor", "zed", "editor", "ide"]),
                ),
            )
            .group(
                SettingGroup::new()
                    .title("Updates")
                    .item(
                        SettingItem::new(
                            "Check for updates automatically",
                            switch(|p| p.check_updates, |p, v| p.check_updates = v)
                                .default_value(defaults.check_updates),
                        )
                        .description("Looks for a newer release on GitHub at launch and every six hours."),
                    )
                    .item(
                        SettingItem::new(
                            "Download and install updates automatically",
                            switch(|p| p.auto_update, |p, v| p.auto_update = v)
                                .default_value(defaults.auto_update),
                        )
                        .description(
                            "Downloads and verifies new versions in the background. They install when you restart or quit.",
                        ),
                    ),
            );

        let mut providers = SettingPage::new("Providers")
            .icon(IconName::Bot)
            .resettable(true);
        for kind in ProviderKind::ALL {
            providers = providers.group(provider_group(kind, cx));
        }

        vec![
            general,
            appearance,
            code,
            terminal,
            providers,
            external_page(),
        ]
    }
}

fn external_page() -> SettingPage {
    SettingPage::new("Agents & MCP")
        .icon(IconName::Network)
        .group(
            SettingGroup::new()
                .title("Agent gateway")
                .item(
                    SettingItem::new(
                        "Let agents manage threads",
                        switch(|p| p.agent_gateway, |p, v| p.agent_gateway = v)
                            .default_value(false),
                    )
                    .description(
                        "Gives Claude Code, Codex, Elyra and ACP agents MCP tools to list, read, create, message and wait for threads, so one agent can fan out work, and to look at local pages in the thread's browser. Applies to agents started afterwards.",
                    ),
                )
                .item(SettingItem::new(
                    "Server",
                    SettingField::render(|_, _, cx| {
                        div()
                            .text_sm()
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_color(cx.theme().muted_foreground)
                            .child(crate::gateway::url(cx).unwrap_or_else(|| "not running".into()))
                    }),
                )),
        )
        .group(
            SettingGroup::new().title("Thread goals").item(
                SettingItem::new(
                    "Automatic turns per goal",
                    number_field(number(1., 100., 1.), |p| p.goal_max_turns as f64, |p, v| {
                        p.goal_max_turns = v.max(1.).round() as u32
                    })
                    .default_value(10.),
                )
                .description("A goal pauses after this many turns without being achieved."),
            ),
        )
        .group(
            SettingGroup::new()
                .title("External clients")
                .description(
                    "Pair Claude Desktop, Codex or another MCP client to control Elyra Workspace. Each client gets its own token; revoke it any time.",
                )
                .item(SettingItem::new(
                    "Paired clients",
                    SettingField::render(|_, _, cx| render_clients(cx)),
                )),
        )
        .group(
            SettingGroup::new()
                .title("Activity")
                .description("Every gateway call, newest first.")
                .item(SettingItem::new(
                    "Audit log",
                    SettingField::render(|_, _, cx| render_audit(cx)),
                )),
        )
}

fn render_clients(cx: &mut App) -> AnyElement {
    let Some(app) = preferences::app_state(cx) else {
        return div().into_any_element();
    };
    let clients = app.read(cx).store.mcp_clients().unwrap_or_default();
    let url = crate::gateway::url(cx).unwrap_or_default();
    let copy = |id: String, label: &'static str, text: String| {
        Button::new(SharedString::from(id))
            .xsmall()
            .ghost()
            .label(label)
            .on_click(move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))
            })
    };
    let rows = clients.into_iter().map(|client| {
        let id = client.id;
        let used = client
            .last_used_at
            .map(|t| {
                format!(
                    "used {} ago",
                    crate::workspace::format_age(chrono::Utc::now() - t)
                )
            })
            .unwrap_or_else(|| "never used".into());
        let scope = match client.scope {
            elyra_core::ClientScope::Full => "full access",
            elyra_core::ClientScope::ReadOnly => "read-only",
        };
        let revoke_app = app.clone();
        v_flex()
            .gap_1()
            .py_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .gap_2()
                    .child(div().text_sm().child(client.name.clone()))
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("{scope} · {used}")),
                    )
                    .child(
                        Button::new(SharedString::from(format!("revoke-{id}")))
                            .xsmall()
                            .ghost()
                            .icon(IconName::Trash)
                            .tooltip("Revoke")
                            .on_click(move |_, _, cx| {
                                crate::gateway::revoke_client(&revoke_app, id, cx)
                            }),
                    ),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(copy(
                        format!("copy-desktop-{id}"),
                        "Copy Claude Desktop config",
                        crate::gateway::client_config("claude-desktop", &url, &client.token),
                    ))
                    .child(copy(
                        format!("copy-codex-{id}"),
                        "Copy Codex config",
                        crate::gateway::client_config("codex", &url, &client.token),
                    ))
                    .child(copy(
                        format!("copy-cc-{id}"),
                        "Copy Claude Code command",
                        crate::gateway::client_config("claude-code", &url, &client.token),
                    )),
            )
    });
    let (read_app, full_app) = (app.clone(), app.clone());
    v_flex()
        .w_full()
        .gap_1()
        .children(rows)
        .child(
            h_flex()
                .gap_2()
                .pt_1()
                .child(
                    Button::new("pair-read")
                        .small()
                        .label("Pair read-only client")
                        .on_click(move |_, _, cx| {
                            crate::gateway::pair_client(
                                &read_app,
                                elyra_core::ClientScope::ReadOnly,
                                cx,
                            );
                        }),
                )
                .child(
                    Button::new("pair-full")
                        .small()
                        .label("Pair full-access client")
                        .on_click(move |_, _, cx| {
                            crate::gateway::pair_client(
                                &full_app,
                                elyra_core::ClientScope::Full,
                                cx,
                            );
                        }),
                ),
        )
        .into_any_element()
}

fn render_audit(cx: &mut App) -> AnyElement {
    let Some(app) = preferences::app_state(cx) else {
        return div().into_any_element();
    };
    let entries = app.read(cx).store.audit_log(40).unwrap_or_default();
    if entries.is_empty() {
        return div()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child("No calls yet.")
            .into_any_element();
    }
    v_flex()
        .w_full()
        .children(entries.into_iter().map(|entry| {
            h_flex()
                .gap_2()
                .text_xs()
                .child(
                    div()
                        .w(px(110.))
                        .flex_none()
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            entry
                                .at
                                .with_timezone(&chrono::Local)
                                .format("%d.%m %H:%M:%S")
                                .to_string(),
                        ),
                )
                .child(
                    div()
                        .w(px(150.))
                        .flex_none()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(entry.client),
                )
                .child(
                    div()
                        .w(px(120.))
                        .flex_none()
                        .text_color(if entry.ok {
                            cx.theme().foreground
                        } else {
                            cx.theme().danger
                        })
                        .child(entry.tool),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .font_family(cx.theme().mono_font_family.clone())
                        .text_color(cx.theme().muted_foreground)
                        .child(entry.detail),
                )
        }))
        .into_any_element()
}

fn provider_field(
    kind: ProviderKind,
    get: impl Fn(&ProviderSettings) -> String + 'static,
    set: impl Fn(&mut ProviderSettings, String) + 'static,
) -> SettingField<SharedString> {
    SettingField::input(
        move |cx| get(&Preferences::global(cx).provider(kind)).into(),
        move |value, cx| {
            preferences::update(cx, |prefs| {
                let entry = prefs
                    .providers
                    .entry(kind.as_str().to_string())
                    .or_default();
                set(entry, value.trim().to_string());
            })
        },
    )
    .default_value(SharedString::default())
}

fn provider_group(kind: ProviderKind, cx: &App) -> SettingGroup {
    let custom = kind == ProviderKind::CustomAcp;
    let acp = elyra_provider::acp::is_acp(kind);
    let status = cx
        .try_global::<ProviderStatus>()
        .and_then(|status| status.0.get(&kind).cloned());
    let install = elyra_provider::install_info(kind);
    let mut group = SettingGroup::new()
        .title(kind.label())
        .item(SettingItem::new(
            "Status",
            SettingField::render(move |_, _, cx| {
                let (text, ok) = match &status {
                    None => ("Checking…".to_string(), false),
                    Some(Some(version)) => (format!("Installed · {version}"), true),
                    Some(None) if custom => ("Set the command below".to_string(), false),
                    Some(None) => (
                        format!("Not found · {}", install.map(|i| i.1).unwrap_or("")),
                        false,
                    ),
                };
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(if ok {
                                cx.theme().success
                            } else {
                                cx.theme().muted_foreground
                            })
                            .child(text),
                    )
                    .when(elyra_provider::login_command(kind).is_some(), |this| {
                        this.child(
                            Button::new(SharedString::from(format!("sign-in-{}", kind.as_str())))
                                .small()
                                .label("Sign in…")
                                .on_click(move |_, _, cx| sign_in(kind, cx)),
                        )
                    })
            }),
        ))
        .item(
            SettingItem::new(
                "Enabled",
                SettingField::switch(
                    move |cx| !Preferences::global(cx).provider(kind).disabled,
                    move |value, cx| {
                        preferences::update(cx, |prefs| {
                            prefs
                                .providers
                                .entry(kind.as_str().to_string())
                                .or_default()
                                .disabled = !value;
                        })
                    },
                )
                .default_value(true),
            )
            .description("Offer this provider for new threads."),
        )
        .item(
            SettingItem::new(
                if custom { "Command" } else { "Executable" },
                provider_field(kind, |p| p.path.clone(), |p, v| p.path = v),
            )
            .description(if custom {
                "Path of an agent that speaks the Agent Client Protocol over stdio."
            } else {
                "Leave empty to find it on PATH."
            }),
        );
    if acp {
        group = group.item(
            SettingItem::new(
                "Arguments",
                provider_field(kind, |p| p.args.clone(), |p, v| p.args = v),
            )
            .description(match elyra_provider::acp::agent(kind) {
                Some(agent) => format!("Leave empty for `{}`.", agent.args.join(" ")),
                None => "For example `--acp`.".to_string(),
            }),
        );
    }
    group
        .item(
            SettingItem::new(
                "Environment",
                provider_field(kind, |p| p.env.clone(), |p, v| p.env = v),
            )
            .description("Extra variables, e.g. `API_KEY=… HTTPS_PROXY=…`."),
        )
        .item(
            SettingItem::new(
                "Accounts",
                provider_field(kind, |p| p.accounts.clone(), |p, v| p.accounts = v),
            )
            .description(
                "Named environments to pick per thread, e.g. `work: CLAUDE_CONFIG_DIR=~/.claude-work; personal: CLAUDE_CONFIG_DIR=~/.claude`.",
            ),
        )
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                TitleBar::new().child(
                    div()
                        .w_full()
                        .text_sm()
                        .text_center()
                        .text_color(cx.theme().muted_foreground)
                        .child("Settings"),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(Settings::new("elyra-settings").pages(Self::pages(cx))),
            )
    }
}

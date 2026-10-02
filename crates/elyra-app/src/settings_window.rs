//! The Settings window (⌘,): appearance, fonts and terminal behavior.

use crate::preferences::{self, Preferences};
use crate::themes;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::setting::{
    NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings,
};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, TitleBar, v_flex};
use gpui_kit::*;

struct SettingsWindow(Option<AnyWindowHandle>);

impl Global for SettingsWindow {}

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
                SettingGroup::new().title("Updates").item(
                    SettingItem::new(
                        "Check for updates automatically",
                        switch(|p| p.check_updates, |p, v| p.check_updates = v)
                            .default_value(defaults.check_updates),
                    )
                    .description("Looks for a newer release on GitHub at launch."),
                ),
            );

        vec![general, appearance, code, terminal]
    }
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

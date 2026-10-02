//! Built-in color themes. Each theme defines the UI colors (registered with
//! gpui-component's theme registry) and a matching terminal palette.

use elyra_terminal::Palette;
use gpui_kit::App;
use gpui_kit::component::ThemeRegistry;
use serde_json::{Map, Value, json};

pub const DEFAULT_DARK: &str = "Default Dark";
pub const DEFAULT_LIGHT: &str = "Default Light";

/// A compact theme description; the full gpui-component color map is
/// derived from it.
struct ThemeDef {
    name: &'static str,
    dark: bool,
    /// Editor/window background.
    background: &'static str,
    /// Sidebar, title bar and tab bar background.
    chrome: &'static str,
    /// Raised surfaces: muted, secondary, popovers, inputs.
    surface: &'static str,
    /// Hover and selected rows.
    surface_hover: &'static str,
    border: &'static str,
    foreground: &'static str,
    muted_foreground: &'static str,
    accent: &'static str,
    accent_foreground: &'static str,
    selection: &'static str,
    red: &'static str,
    green: &'static str,
    yellow: &'static str,
    blue: &'static str,
    magenta: &'static str,
    cyan: &'static str,
    terminal: Palette,
}

const THEMES: &[ThemeDef] = &[
    ThemeDef {
        name: "Tokyo Night",
        dark: true,
        background: "#1a1b26",
        chrome: "#16161e",
        surface: "#24283b",
        surface_hover: "#292e42",
        border: "#2a2e42",
        foreground: "#c0caf5",
        muted_foreground: "#737aa2",
        accent: "#7aa2f7",
        accent_foreground: "#16161e",
        selection: "#33467c",
        red: "#f7768e",
        green: "#9ece6a",
        yellow: "#e0af68",
        blue: "#7aa2f7",
        magenta: "#bb9af7",
        cyan: "#7dcfff",
        terminal: Palette::new(
            0xc0caf5,
            0x1a1b26,
            [
                0x15161e, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xa9b1d6,
                0x414868, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xc0caf5,
            ],
        ),
    },
    ThemeDef {
        name: "Palenight",
        dark: true,
        background: "#292d3e",
        chrome: "#232635",
        surface: "#32374d",
        surface_hover: "#3a3f58",
        border: "#3a3f58",
        foreground: "#bfc7d5",
        muted_foreground: "#7982a9",
        accent: "#c792ea",
        accent_foreground: "#232635",
        selection: "#444267",
        red: "#f07178",
        green: "#c3e88d",
        yellow: "#ffcb6b",
        blue: "#82aaff",
        magenta: "#c792ea",
        cyan: "#89ddff",
        terminal: Palette::new(
            0xbfc7d5,
            0x292d3e,
            [
                0x292d3e, 0xf07178, 0xc3e88d, 0xffcb6b, 0x82aaff, 0xc792ea, 0x89ddff, 0xd0d0d0,
                0x434758, 0xff8b92, 0xddffa7, 0xffe585, 0x9cc4ff, 0xe1acff, 0xa3f7ff, 0xffffff,
            ],
        ),
    },
    ThemeDef {
        name: "Dracula",
        dark: true,
        background: "#282a36",
        chrome: "#21222c",
        surface: "#343746",
        surface_hover: "#44475a",
        border: "#3a3c4e",
        foreground: "#f8f8f2",
        muted_foreground: "#8b93c4",
        accent: "#bd93f9",
        accent_foreground: "#21222c",
        selection: "#44475a",
        red: "#ff5555",
        green: "#50fa7b",
        yellow: "#f1fa8c",
        blue: "#bd93f9",
        magenta: "#ff79c6",
        cyan: "#8be9fd",
        terminal: Palette::new(
            0xf8f8f2,
            0x282a36,
            [
                0x21222c, 0xff5555, 0x50fa7b, 0xf1fa8c, 0xbd93f9, 0xff79c6, 0x8be9fd, 0xf8f8f2,
                0x6272a4, 0xff6e6e, 0x69ff94, 0xffffa5, 0xd6acff, 0xff92df, 0xa4ffff, 0xffffff,
            ],
        ),
    },
    ThemeDef {
        name: "Nord",
        dark: true,
        background: "#2e3440",
        chrome: "#272c36",
        surface: "#3b4252",
        surface_hover: "#434c5e",
        border: "#3b4252",
        foreground: "#d8dee9",
        muted_foreground: "#7b88a1",
        accent: "#88c0d0",
        accent_foreground: "#2e3440",
        selection: "#434c5e",
        red: "#bf616a",
        green: "#a3be8c",
        yellow: "#ebcb8b",
        blue: "#81a1c1",
        magenta: "#b48ead",
        cyan: "#88c0d0",
        terminal: Palette::new(
            0xd8dee9,
            0x2e3440,
            [
                0x3b4252, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x88c0d0, 0xe5e9f0,
                0x4c566a, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x8fbcbb, 0xeceff4,
            ],
        ),
    },
];

fn colors(def: &ThemeDef) -> Map<String, Value> {
    let alpha = |hex: &str, a: &str| format!("{hex}{a}");
    let pairs = [
        ("background", def.background.to_string()),
        ("foreground", def.foreground.to_string()),
        ("border", def.border.to_string()),
        ("input.border", def.surface_hover.to_string()),
        ("ring", def.accent.to_string()),
        ("caret", def.accent.to_string()),
        ("selection.background", def.selection.to_string()),
        ("accent.background", def.surface_hover.to_string()),
        ("accent.foreground", def.foreground.to_string()),
        ("muted.background", def.surface.to_string()),
        ("muted.foreground", def.muted_foreground.to_string()),
        ("primary.background", def.accent.to_string()),
        ("primary.hover.background", alpha(def.accent, "e6")),
        ("primary.active.background", alpha(def.accent, "cc")),
        ("primary.foreground", def.accent_foreground.to_string()),
        ("secondary.background", def.surface.to_string()),
        ("secondary.hover.background", def.surface_hover.to_string()),
        ("secondary.active.background", def.surface_hover.to_string()),
        ("secondary.foreground", def.foreground.to_string()),
        ("popover.background", def.surface.to_string()),
        ("popover.foreground", def.foreground.to_string()),
        ("accordion.background", def.background.to_string()),
        ("group_box.background", def.chrome.to_string()),
        ("group_box.foreground", def.foreground.to_string()),
        ("list.background", def.background.to_string()),
        ("list.hover.background", def.surface.to_string()),
        ("list.active.background", alpha(def.accent, "33")),
        ("list.active.border", def.accent.to_string()),
        ("list.even.background", def.background.to_string()),
        ("list.head.background", def.chrome.to_string()),
        ("sidebar.background", def.chrome.to_string()),
        ("sidebar.foreground", def.foreground.to_string()),
        ("sidebar.border", def.border.to_string()),
        ("sidebar.accent.background", def.surface_hover.to_string()),
        ("sidebar.accent.foreground", def.foreground.to_string()),
        ("sidebar.primary.background", def.accent.to_string()),
        (
            "sidebar.primary.foreground",
            def.accent_foreground.to_string(),
        ),
        ("title_bar.background", def.chrome.to_string()),
        ("title_bar.border", def.border.to_string()),
        ("tab_bar.background", def.chrome.to_string()),
        ("tab_bar.segmented.background", def.chrome.to_string()),
        ("tab.background", "#00000000".to_string()),
        ("tab.foreground", def.muted_foreground.to_string()),
        ("tab.active.background", def.background.to_string()),
        ("tab.active.foreground", def.foreground.to_string()),
        ("status_bar.background", def.chrome.to_string()),
        ("status_bar.border", def.border.to_string()),
        ("table.background", def.background.to_string()),
        ("table.head.foreground", def.muted_foreground.to_string()),
        ("table.row.border", def.border.to_string()),
        ("scrollbar.background", "#00000000".to_string()),
        (
            "scrollbar.thumb.background",
            alpha(def.muted_foreground, "66"),
        ),
        (
            "scrollbar.thumb.hover.background",
            alpha(def.muted_foreground, "99"),
        ),
        ("skeleton.background", def.surface.to_string()),
        ("switch.background", def.surface_hover.to_string()),
        ("slider.bar.background", def.accent.to_string()),
        ("slider.thumb.background", def.foreground.to_string()),
        ("progress_bar.background", def.accent.to_string()),
        ("link.foreground", def.blue.to_string()),
        ("link.hover.foreground", def.cyan.to_string()),
        ("link.active.foreground", def.blue.to_string()),
        ("drag_border", def.accent.to_string()),
        ("drop_target.background", alpha(def.accent, "33")),
        ("window.border", def.border.to_string()),
        ("overlay", "#00000055".to_string()),
        ("danger.background", def.red.to_string()),
        ("danger.foreground", def.red.to_string()),
        ("success.background", def.green.to_string()),
        ("success.foreground", def.green.to_string()),
        ("warning.background", def.yellow.to_string()),
        ("warning.foreground", def.yellow.to_string()),
        ("info.background", def.cyan.to_string()),
        ("info.foreground", def.cyan.to_string()),
        ("base.red", def.red.to_string()),
        ("base.green", def.green.to_string()),
        ("base.blue", def.blue.to_string()),
        ("base.yellow", def.yellow.to_string()),
        ("base.magenta", def.magenta.to_string()),
        ("base.cyan", def.cyan.to_string()),
    ];
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_string(), Value::String(value)))
        .collect()
}

fn theme_set() -> Value {
    let themes: Vec<Value> = THEMES
        .iter()
        .map(|def| {
            json!({
                "name": def.name,
                "mode": if def.dark { "dark" } else { "light" },
                "colors": colors(def),
            })
        })
        .collect();
    json!({ "name": "Elyra", "author": "Elyra Workspace", "themes": themes })
}

pub fn register(cx: &mut App) {
    let json = theme_set().to_string();
    if let Err(err) = ThemeRegistry::global_mut(cx).load_themes_from_str(&json) {
        log::error!("registering built-in themes: {err:#}");
    }
}

/// Theme names offered in Settings, in display order.
pub fn names() -> Vec<&'static str> {
    let mut names = vec![DEFAULT_DARK, DEFAULT_LIGHT];
    names.extend(THEMES.iter().map(|def| def.name));
    names
}

pub fn is_dark(name: &str) -> bool {
    name != DEFAULT_LIGHT
        && THEMES
            .iter()
            .find(|def| def.name == name)
            .is_none_or(|def| def.dark)
}

/// The terminal palette matching a UI theme.
pub fn terminal_palette(name: &str) -> Palette {
    match THEMES.iter().find(|def| def.name == name) {
        Some(def) => def.terminal,
        None if name == DEFAULT_LIGHT => Palette::light(),
        None => Palette::dark(),
    }
}

#[cfg(test)]
mod tests {
    use super::{THEMES, colors, is_dark, names, terminal_palette};

    #[test]
    fn themes_are_complete_and_parse() {
        assert_eq!(names().len(), 2 + THEMES.len());
        for def in THEMES {
            let colors = colors(def);
            for value in colors.values() {
                let hex = value.as_str().unwrap();
                assert!(
                    hex.starts_with('#') && matches!(hex.len(), 7 | 9),
                    "{} {hex}",
                    def.name
                );
            }
        }
        assert!(is_dark("Nord") && is_dark("Default Dark") && !is_dark("Default Light"));
        assert_eq!(terminal_palette("Dracula").background.to_u32(), 0x282a36);
    }
}

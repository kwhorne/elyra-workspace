//! User preferences: persisted as JSON in the settings table, held as a GPUI
//! global, and applied to the component theme on every change.

use crate::app_state::AppState;
use crate::themes;
use elyra_terminal::{CursorShape, TerminalOptions};
use gpui_kit::component::{Theme, ThemeRegistry};
use gpui_kit::{App, Global, SharedString, WeakEntity, px};
use serde::{Deserialize, Serialize};

const KEY: &str = "preferences";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub theme: String,
    /// Remembered dark theme for the light/dark toggle.
    pub dark_theme: String,
    /// Empty means the system UI font.
    pub ui_font_family: String,
    pub ui_font_size: f32,
    pub mono_font_family: String,
    pub mono_font_size: f32,
    /// Empty means "same as code font".
    pub terminal_font_family: String,
    pub terminal_font_size: f32,
    pub terminal_line_height: f32,
    pub terminal_cursor_shape: String,
    pub terminal_cursor_blink: bool,
    pub terminal_option_as_meta: bool,
    pub terminal_copy_on_select: bool,
    pub terminal_scrollback: usize,
    /// Empty means the login shell.
    pub terminal_shell: String,
    pub check_updates: bool,
    /// External editor name (see `editors::EDITORS`); empty picks the first
    /// installed one.
    pub editor: String,
    /// Switch between `light_theme` and `dark_theme` with macOS appearance.
    pub follow_system: bool,
    pub light_theme: String,
    /// Maximum width of the conversation column in points; 0 is unlimited.
    pub chat_width: f32,
    /// "comfortable" or "compact".
    pub density: String,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: themes::DEFAULT_DARK.into(),
            dark_theme: themes::DEFAULT_DARK.into(),
            ui_font_family: String::new(),
            ui_font_size: 14.,
            mono_font_family: default_mono_font().into(),
            mono_font_size: 13.,
            terminal_font_family: String::new(),
            terminal_font_size: 13.,
            terminal_line_height: 1.3,
            terminal_cursor_shape: "block".into(),
            terminal_cursor_blink: true,
            terminal_option_as_meta: false,
            terminal_copy_on_select: false,
            terminal_scrollback: 10_000,
            terminal_shell: String::new(),
            check_updates: true,
            editor: String::new(),
            follow_system: false,
            light_theme: themes::DEFAULT_LIGHT.into(),
            chat_width: 860.,
            density: "comfortable".into(),
        }
    }
}

fn default_mono_font() -> &'static str {
    if cfg!(target_os = "macos") {
        "Menlo"
    } else if cfg!(windows) {
        "Consolas"
    } else {
        "DejaVu Sans Mono"
    }
}

impl Global for Preferences {}

impl Preferences {
    pub fn global(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    pub fn terminal_font_family(&self) -> &str {
        if self.terminal_font_family.is_empty() {
            &self.mono_font_family
        } else {
            &self.terminal_font_family
        }
    }

    pub fn editor(&self) -> Option<crate::editors::EditorApp> {
        let installed = crate::editors::installed();
        crate::editors::by_name(&self.editor)
            .filter(|editor| installed.contains(editor))
            .or_else(|| {
                installed
                    .into_iter()
                    .find(|e| e.app != "Finder" && e.app != "Terminal")
            })
    }

    pub fn compact(&self) -> bool {
        self.density == "compact"
    }

    pub fn terminal_options(&self) -> TerminalOptions {
        TerminalOptions {
            shell: Some(self.terminal_shell.clone()).filter(|shell| !shell.trim().is_empty()),
            scrollback: self.terminal_scrollback,
            cursor_shape: match self.terminal_cursor_shape.as_str() {
                "beam" => CursorShape::Beam,
                "underline" => CursorShape::Underline,
                _ => CursorShape::Block,
            },
            cursor_blink: self.terminal_cursor_blink,
        }
    }
}

/// Where preference changes are persisted.
struct PreferencesStore(WeakEntity<AppState>);

impl Global for PreferencesStore {}

/// Load preferences (migrating the old light/dark `theme` setting), install
/// them as a global and apply them.
pub fn init(app: &gpui_kit::Entity<AppState>, cx: &mut App) {
    let state = app.read(cx);
    let stored = state.store.setting(KEY).ok().flatten();
    let legacy_theme = state.store.setting("theme").ok().flatten();
    let mut prefs = stored
        .and_then(|json| serde_json::from_str::<Preferences>(&json).ok())
        .unwrap_or_else(|| {
            let mut prefs = Preferences::default();
            if legacy_theme.as_deref() == Some("light") {
                prefs.theme = themes::DEFAULT_LIGHT.into();
            }
            prefs
        });
    let known = themes::all_names(cx);
    if !known.contains(&prefs.theme) {
        prefs.theme = themes::DEFAULT_DARK.into();
    }
    if !known.contains(&prefs.light_theme) {
        prefs.light_theme = themes::DEFAULT_LIGHT.into();
    }
    cx.set_global(PreferencesStore(app.downgrade()));
    cx.set_global(prefs);
    apply(cx);
    sync_with_system(cx);
}

/// Change preferences, then persist and apply them.
pub fn update(cx: &mut App, edit: impl FnOnce(&mut Preferences)) {
    let before = Preferences::global(cx).clone();
    let mut prefs = before.clone();
    edit(&mut prefs);
    if prefs.follow_system {
        prefs.theme = if system_is_dark(cx) {
            prefs.dark_theme.clone()
        } else {
            prefs.light_theme.clone()
        };
    }
    if prefs == before {
        return;
    }
    if prefs.theme != before.theme {
        if themes::is_dark(&prefs.theme) {
            prefs.dark_theme = prefs.theme.clone();
        } else {
            prefs.light_theme = prefs.theme.clone();
        }
    }
    let json = serde_json::to_string(&prefs).unwrap_or_default();
    cx.set_global(prefs);
    if let Some(app) = cx.global::<PreferencesStore>().0.upgrade() {
        app.update(cx, |app, _| app.set_setting(KEY, &json));
    }
    apply(cx);
}

/// Push the theme and fonts into the component theme; refreshes all windows.
pub fn apply(cx: &mut App) {
    let prefs = Preferences::global(cx).clone();
    let config = ThemeRegistry::global(cx)
        .themes()
        .get(&SharedString::from(prefs.theme.clone()))
        .cloned()
        .unwrap_or_else(|| {
            if themes::is_dark(&prefs.theme) {
                ThemeRegistry::global(cx).default_dark_theme().clone()
            } else {
                ThemeRegistry::global(cx).default_light_theme().clone()
            }
        });
    Theme::update(cx, |theme| {
        theme.apply_config(&config);
        theme.font_family = if prefs.ui_font_family.is_empty() {
            ".SystemUIFont".into()
        } else {
            prefs.ui_font_family.clone().into()
        };
        theme.font_size = px(prefs.ui_font_size);
        theme.mono_font_family = prefs.mono_font_family.clone().into();
        theme.mono_font_size = px(prefs.mono_font_size);
    });
}

/// Toggle between the remembered light and dark themes.
pub fn toggle_light_dark(cx: &mut App) {
    update(cx, |prefs| {
        prefs.follow_system = false;
        prefs.theme = if themes::is_dark(&prefs.theme) {
            prefs.light_theme.clone()
        } else {
            prefs.dark_theme.clone()
        };
    });
}

/// With "follow system appearance" on, pick the light or dark theme that
/// matches the OS.
pub fn sync_with_system(cx: &mut App) {
    if Preferences::global(cx).follow_system {
        update(cx, |_| {});
    }
}

fn system_is_dark(cx: &App) -> bool {
    matches!(
        cx.window_appearance(),
        gpui_kit::WindowAppearance::Dark | gpui_kit::WindowAppearance::VibrantDark
    )
}

const MONO_CANDIDATES: &[&str] = &[
    "Menlo",
    "Monaco",
    "SF Mono",
    "JetBrains Mono",
    "Fira Code",
    "Fira Mono",
    "Cascadia Code",
    "Cascadia Mono",
    "Source Code Pro",
    "Hack",
    "IBM Plex Mono",
    "Iosevka",
    "Ubuntu Mono",
    "Roboto Mono",
    "Inconsolata",
    "Victor Mono",
    "Berkeley Mono",
    "Geist Mono",
    "Commit Mono",
    "Monaspace Neon",
    "Andale Mono",
    "PT Mono",
    "Courier New",
    "Consolas",
    "DejaVu Sans Mono",
];

const UI_CANDIDATES: &[&str] = &[
    "Inter",
    "SF Pro",
    "SF Pro Text",
    "Helvetica Neue",
    "Avenir Next",
    "Lucida Grande",
    "Geneva",
    "Verdana",
    "Roboto",
    "Open Sans",
    "IBM Plex Sans",
    "Source Sans 3",
    "Geist",
    "Segoe UI",
    "Ubuntu",
    "Noto Sans",
];

fn installed(cx: &App) -> Vec<String> {
    let mut names = cx.text_system().all_font_names();
    names.sort();
    names.dedup();
    names
}

/// Monospace families installed on this machine: well-known candidates plus
/// any family whose name marks it as monospace.
pub fn mono_fonts(cx: &App) -> Vec<String> {
    let installed = installed(cx);
    let mut fonts: Vec<String> = MONO_CANDIDATES
        .iter()
        .filter(|name| installed.iter().any(|font| font == *name))
        .map(|name| name.to_string())
        .collect();
    for font in &installed {
        let lower = font.to_lowercase();
        let looks_mono =
            lower.contains("mono") || lower.ends_with(" code") || lower.contains("nerd font");
        if looks_mono && !font.starts_with('.') && !fonts.contains(font) {
            fonts.push(font.clone());
        }
    }
    let current = Preferences::global(cx).mono_font_family.clone();
    if !current.is_empty() && !fonts.contains(&current) {
        fonts.insert(0, current);
    }
    fonts
}

pub fn ui_fonts(cx: &App) -> Vec<String> {
    let installed = installed(cx);
    UI_CANDIDATES
        .iter()
        .filter(|name| installed.iter().any(|font| font == *name))
        .map(|name| name.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::Preferences;

    #[test]
    fn preferences_roundtrip_and_fill_missing_fields() {
        let prefs = Preferences::default();
        let json = serde_json::to_string(&prefs).unwrap();
        assert_eq!(serde_json::from_str::<Preferences>(&json).unwrap(), prefs);
        let partial: Preferences = serde_json::from_str(r#"{"theme":"Nord"}"#).unwrap();
        assert_eq!(partial.theme, "Nord");
        assert_eq!(partial.terminal_scrollback, 10_000);
        assert_eq!(partial.terminal_font_family(), partial.mono_font_family);
    }
}

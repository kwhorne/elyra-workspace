//! The keyboard shortcuts sheet (⌘/).

use crate::actions::{self, display_keys};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::*;

pub fn show(window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, |dialog, _, cx| {
        let mut groups: Vec<(&str, Vec<(String, String)>)> = Vec::new();
        for (shortcut, keys) in actions::effective_keys() {
            if shortcut.label.is_empty() {
                continue;
            }
            let keys = keys.map(|k| display_keys(&k)).unwrap_or_else(|| "—".into());
            let keys = if shortcut.id == "select_tab_1" {
                "⌘1 … ⌘9".to_string()
            } else {
                keys
            };
            match groups
                .iter_mut()
                .find(|(group, _)| *group == shortcut.group)
            {
                Some((_, rows)) => rows.push((shortcut.label.to_string(), keys)),
                None => groups.push((shortcut.group, vec![(shortcut.label.to_string(), keys)])),
            }
        }
        let columns = groups.into_iter().map(|(group, rows)| {
            v_flex()
                .gap_1()
                .mb_3()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(cx.theme().muted_foreground)
                        .child(group.to_uppercase()),
                )
                .children(rows.into_iter().map(|(label, keys)| {
                    h_flex()
                        .gap_4()
                        .text_sm()
                        .child(div().flex_1().child(label))
                        .child(
                            div()
                                .px_1p5()
                                .rounded_sm()
                                .bg(cx.theme().muted)
                                .font_family(cx.theme().mono_font_family.clone())
                                .text_xs()
                                .child(keys),
                        )
                }))
        });
        let path = actions::keybindings_path();
        dialog
            .title("Keyboard shortcuts")
            .w(px(720.))
            .child(
                div()
                    .id("shortcuts")
                    .max_h(px(520.))
                    .overflow_y_scroll()
                    .child(div().grid().grid_cols(2).gap_x_8().children(columns)),
            )
            .footer(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!(
                                "Override shortcuts in {} (restart to apply).",
                                path.display()
                            )),
                    )
                    .child(
                        Button::new("edit-keybindings")
                            .small()
                            .ghost()
                            .icon(IconName::Pencil)
                            .label("Edit…")
                            .on_click(move |_, _, cx| {
                                if !path.exists() {
                                    // Start from the defaults so every id is discoverable.
                                    let defaults: serde_json::Map<String, serde_json::Value> =
                                        actions::SHORTCUTS
                                            .iter()
                                            .map(|s| (s.id.to_string(), s.keys.into()))
                                            .collect();
                                    let json =
                                        serde_json::to_string_pretty(&defaults).unwrap_or_default();
                                    let _ = std::fs::write(&path, json);
                                }
                                cx.open_with_system(&path);
                            }),
                    ),
            )
    });
}

//! The About dialog, styled like Elyra Conductor's: logo, name, version,
//! tagline, link cards and a Close button over a dimmed backdrop.

use crate::app_icon::{BRAND_AMBER, BRAND_YELLOW, ICON_PNG};
use crate::workspace::Workspace;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::*;
use std::sync::Arc;

const LINKS: &[(&str, &str, &str)] = &[
    (
        "Website",
        "elyracode.com/workspace",
        "https://elyracode.com/workspace",
    ),
    (
        "GitHub",
        "github.com/kwhorne/elyra-workspace",
        "https://github.com/kwhorne/elyra-workspace",
    ),
    (
        "Developed by",
        "Knut W. Horne · kwhorne.com",
        "https://kwhorne.com/",
    ),
];

pub fn logo() -> Arc<Image> {
    Arc::new(Image::from_bytes(ImageFormat::Png, ICON_PNG.to_vec()))
}

pub fn render(logo: Arc<Image>, focus: &FocusHandle, cx: &Context<Workspace>) -> AnyElement {
    let theme = cx.theme();
    let accent: Hsla = if theme.is_dark() {
        rgb(BRAND_YELLOW).into()
    } else {
        rgb(BRAND_AMBER).into()
    };
    let mono = theme.mono_font_family.clone();

    let links: Vec<_> = LINKS
        .iter()
        .enumerate()
        .map(|(index, (label, text, url))| {
            let url = url.to_string();
            v_flex()
                .id(("about-link", index))
                .gap(px(1.))
                .px_3()
                .py_2()
                .rounded(px(9.))
                .bg(theme.muted)
                .border_1()
                .border_color(theme.border)
                .cursor_pointer()
                .hover(move |style| style.border_color(accent))
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(theme.muted_foreground)
                        .child(label.to_uppercase()),
                )
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(accent)
                        .font_family(mono.clone())
                        .child(*text),
                )
                .on_click(move |_, _, cx| cx.open_url(&url))
        })
        .collect();

    div()
        .id("about-overlay")
        .track_focus(focus)
        .key_context("AboutDialog")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(hsla(0., 0., 0., 0.5))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| this.close_about(cx)),
        )
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
            if matches!(event.keystroke.key.as_str(), "escape" | "enter") {
                this.close_about(cx);
                cx.stop_propagation();
            }
        }))
        .child(
            v_flex()
                .id("about-dialog")
                .w(px(380.))
                .px(px(28.))
                .py(px(26.))
                .items_center()
                .rounded(px(14.))
                .bg(theme.popover)
                .text_color(theme.popover_foreground)
                .border_1()
                .border_color(theme.border)
                .shadow_2xl()
                // Clicks inside the dialog must not reach the backdrop.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(img(logo).size(px(56.)).mb(px(10.)))
                .child(
                    div()
                        .text_size(px(18.))
                        .font_weight(FontWeight::BOLD)
                        .child("Elyra Workspace"),
                )
                .child(
                    div()
                        .mt(px(2.))
                        .text_size(px(12.))
                        .font_family(mono.clone())
                        .text_color(theme.muted_foreground)
                        .child(format!("Version {}", env!("CARGO_PKG_VERSION"))),
                )
                .child(
                    div()
                        .mt(px(14.))
                        .mb(px(18.))
                        .text_size(px(12.))
                        .line_height(relative(1.5))
                        .text_center()
                        .text_color(theme.muted_foreground)
                        .child(
                            "A focused workspace for coding agents — projects, threads, \
                             agent chat, changes, Git and a real terminal.",
                        ),
                )
                .child(v_flex().w_full().gap(px(6.)).children(links))
                .child(
                    h_flex()
                        .id("about-close")
                        .mt(px(18.))
                        .px(px(18.))
                        .py(px(7.))
                        .rounded(px(8.))
                        .bg(theme.muted)
                        .border_1()
                        .border_color(theme.border)
                        .text_size(px(12.))
                        .cursor_pointer()
                        .hover(move |style| style.border_color(accent))
                        .child("Close")
                        .on_click(cx.listener(|this, _, _, cx| this.close_about(cx))),
                ),
        )
        .into_any_element()
}

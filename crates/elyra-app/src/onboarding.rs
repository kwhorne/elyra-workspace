//! First-run welcome: provider setup status, theme and first project.

use crate::app_state::AppState;
use crate::preferences::{self, Preferences};
use crate::themes;
use crate::workspace::Workspace;
use elyra_core::ProviderKind;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

const INSTALL_HINTS: [(ProviderKind, &str); 2] = [
    (
        ProviderKind::Claude,
        "npm install -g @anthropic-ai/claude-code && claude",
    ),
    (
        ProviderKind::Elyra,
        "npm install -g @elyracode/coding-agent",
    ),
];

pub struct OnboardingView {
    versions: Vec<(ProviderKind, Option<String>)>,
    checking: bool,
}

fn cli_version(kind: ProviderKind) -> Option<String> {
    let executable = match kind {
        ProviderKind::Claude => elyra_provider::claude::find_executable(),
        ProviderKind::Elyra => elyra_provider::elyra::find_executable(),
    }?;
    let output = std::process::Command::new(executable)
        .arg("--version")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    Some(text.lines().next().unwrap_or("").trim().to_string()).filter(|v| !v.is_empty())
}

impl OnboardingView {
    fn new(cx: &mut Context<Self>) -> Self {
        let job = cx.background_executor().spawn(async move {
            ProviderKind::ALL
                .into_iter()
                .map(|kind| (kind, cli_version(kind)))
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let versions = job.await;
            let _ = this.update(cx, |this, cx| {
                this.versions = versions;
                this.checking = false;
                cx.notify();
            });
        })
        .detach();
        Self {
            versions: Vec::new(),
            checking: true,
        }
    }
}

impl Render for OnboardingView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current_theme = Preferences::global(cx).theme.clone();
        let providers = INSTALL_HINTS.iter().map(|(kind, hint)| {
            let version = self
                .versions
                .iter()
                .find(|(k, _)| k == kind)
                .and_then(|(_, v)| v.clone());
            let installed = version.is_some();
            h_flex()
                .gap_3()
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(cx.theme().border)
                .child(
                    Icon::new(if self.checking {
                        IconName::LoaderCircle
                    } else if installed {
                        IconName::CircleCheck
                    } else {
                        IconName::CircleDashed
                    })
                    .small()
                    .text_color(if installed {
                        cx.theme().success
                    } else {
                        cx.theme().muted_foreground
                    }),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .child(kind.label()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .when(installed, |this| {
                                    this.child(format!(
                                        "Installed · {}",
                                        version.clone().unwrap_or_default()
                                    ))
                                })
                                .when(!installed && !self.checking, |this| {
                                    this.font_family(cx.theme().mono_font_family.clone())
                                        .child(hint.to_string())
                                }),
                        ),
                )
        });
        let theme_buttons = themes::names().into_iter().map(|name| {
            let selected = current_theme == name;
            Button::new(SharedString::from(format!("onboard-theme-{name}")))
                .xsmall()
                .when(selected, |this| this.primary())
                .label(name)
                .on_click(move |_, _, cx| preferences::update(cx, |p| p.theme = name.to_string()))
        });
        v_flex()
            .gap_4()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Elyra Workspace runs coding agents installed on this Mac. Install and sign in to at least one."),
            )
            .child(v_flex().gap_2().children(providers))
            .child(
                v_flex()
                    .gap_2()
                    .child(div().text_sm().font_weight(FontWeight::MEDIUM).child("Theme"))
                    .child(h_flex().flex_wrap().gap_1().children(theme_buttons)),
            )
    }
}

pub fn should_show(app: &Entity<AppState>, cx: &App) -> bool {
    let state = app.read(cx);
    state.projects.is_empty()
        && state
            .store
            .setting("onboarding_done")
            .ok()
            .flatten()
            .is_none()
}

pub fn show(
    workspace: WeakEntity<Workspace>,
    app: Entity<AppState>,
    window: &mut Window,
    cx: &mut App,
) {
    let view = cx.new(OnboardingView::new);
    let done = move |app: &Entity<AppState>, cx: &mut App| {
        app.update(cx, |app, _| app.set_setting("onboarding_done", "1"));
    };
    window.open_dialog(cx, move |dialog, _, _| {
        let (add_app, chat_app, close_app) = (app.clone(), app.clone(), app.clone());
        let (add_workspace, chat_workspace) = (workspace.clone(), workspace.clone());
        dialog
            .title("Welcome to Elyra Workspace")
            .w(px(560.))
            .child(view.clone())
            .on_close(move |_, _, cx| done(&close_app, cx))
            .footer(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("onboard-chat")
                            .small()
                            .label("Start a chat")
                            .on_click(move |_, window, cx| {
                                done(&chat_app, cx);
                                window.close_dialog(cx);
                                let _ =
                                    chat_workspace.update(cx, |this, cx| this.new_chat(window, cx));
                            }),
                    )
                    .child(
                        Button::new("onboard-project")
                            .small()
                            .primary()
                            .icon(IconName::FolderPlus)
                            .label("Add a project")
                            .on_click(move |_, window, cx| {
                                done(&add_app, cx);
                                window.close_dialog(cx);
                                let _ = add_workspace
                                    .update(cx, |this, cx| this.add_project(window, cx));
                            }),
                    ),
            )
    });
}
